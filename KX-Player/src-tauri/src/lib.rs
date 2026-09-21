//! 应用装配：插件/托盘/关窗到托盘/单实例/命令注册。

mod ai;
mod bgimage;
mod commands;
mod db;
mod error;
mod ffprobe;
mod fftools;
mod model;
mod paths;
mod player;
mod scanner;
mod settingsio;
mod state;
mod taxonomy;
mod watcher;

use state::AppWindowsState;
use tauri::tray::TrayIconBuilder;
use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager, WindowEvent};

/// PlayerCore 从 player 模块导出
pub use player::PlayerCore;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .setup(|app| {
            // 数据目录与封面目录（沿用 Electron 版 userData 路径）
            let _ = paths::ensure_data_dir();
            let _ = paths::ensure_covers_dir();
            db::check_integrity();
            let migrated = db::migrate_legacy_ids(&paths::library_db_path(), &paths::settings_path());
            if migrated > 0 {
                paths::append_log(&format!("条目 ID 迁移：{migrated} 条旧 ID 已改写为 UUIDv5，备份见 *.pre-uuid5"));
            }
            scanner::covers::load_folder_cover_map();
            bgimage::migrate_oversized();

            // asset 协议范围：封面目录 + 数据目录（背景图）
            let scope = app.asset_protocol_scope();
            let _ = scope.allow_directory(paths::covers_dir(), true);
            let _ = scope.allow_directory(paths::data_dir(), true);

            state::set_app_handle(app.handle().clone());
            app.manage(AppWindowsState::new());
            app.manage(PlayerCore::new(app.handle().clone()));

            build_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 悬浮窗（pip）事件与主窗口互不干扰，单独分流
            if window.label() == "pip" {
                let app = window.app_handle();
                match event {
                    // 用户 Alt+F4 / 系统菜单关闭：隐藏并挂回主窗口，不真正销毁
                    // （窗口实例保留，再次打开时复用，避免重复创建 WebView 的开销）
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let player: tauri::State<PlayerCore> = app.state();
                        player.detach_overlay();
                        let _ = window.hide();
                        let _ = app.emit("pip:closed", ());
                    }
                    WindowEvent::Destroyed => {
                        let player: tauri::State<PlayerCore> = app.state();
                        player.detach_overlay();
                        let _ = app.emit("pip:closed", ());
                    }
                    // 悬浮窗移动/缩放：覆盖窗口跟随（矩形相对 pip 客户区，重放即换算到新屏幕位置）
                    WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                        let player: tauri::State<PlayerCore> = app.state();
                        player.reposition();
                    }
                    _ => {}
                }
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let state: tauri::State<AppWindowsState> = window.app_handle().state();
                    let player: tauri::State<PlayerCore> = window.app_handle().state();
                    if state.force_close.load(std::sync::atomic::Ordering::SeqCst) {
                        player.destroy_host();
                    } else {
                        api.prevent_close();
                        // 关闭=隐藏到托盘：一并隐藏视频覆盖窗口（悬浮窗挂靠期间不受影响）
                        player.set_stage_visible(false);
                        let _ = window.hide();
                    }
                }
                WindowEvent::Resized { .. } => {
                    let state: tauri::State<AppWindowsState> = window.app_handle().state();
                    let maximized = window.is_maximized().unwrap_or(false);
                    let last = state.last_maximized.load(std::sync::atomic::Ordering::SeqCst);
                    if maximized != last {
                        state.last_maximized.store(maximized, std::sync::atomic::Ordering::SeqCst);
                        let _ = window.app_handle().emit("window:maximizeChange", maximized);
                    }
                    // 缩放/最大化：视频覆盖窗口跟随（客户区矩形不变，但屏幕位置可能已变）
                    let player: tauri::State<PlayerCore> = window.app_handle().state();
                    player.reposition();
                }
                WindowEvent::Moved(_) => {
                    let player: tauri::State<PlayerCore> = window.app_handle().state();
                    player.reposition();
                }
                WindowEvent::Focused(focused) => {
                    let player: tauri::State<PlayerCore> = window.app_handle().state();
                    // 失焦隐藏视频覆盖层（避免浮在别的应用上），聚焦恢复
                    player.focus_changed(*focused);
                }
                WindowEvent::Destroyed => {
                    let player: tauri::State<PlayerCore> = window.app_handle().state();
                    player.destroy_host();
                    // 主窗口真正销毁时一并销毁悬浮窗（应用退出）
                    if let Some(pip) = window.app_handle().get_webview_window("pip") {
                        let _ = pip.destroy();
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::dialog_fs::open_folder,
            commands::dialog_fs::open_image_file,
            commands::dialog_fs::open_audio_files,
            commands::dialog_fs::select_bg_image,
            commands::dialog_fs::read_as_data_url,
            commands::dialog_fs::read_text_file,
            commands::dialog_fs::file_exists,
            commands::dialog_fs::list_dir,
            commands::dialog_fs::rename_dir,
            commands::dialog_fs::tools_save_file,
            commands::dialog_fs::clipboard_write_text,
            commands::dialog_fs::show_item_in_folder,
            commands::library::scan_folders,
            commands::library::scan_folders_incremental,
            commands::library::startup_sync,
            commands::library::remove_folder,
            commands::library::load_library,
            commands::library::load_library_fast,
            commands::library::get_track_covers,
            commands::library::get_folder_covers,
            commands::library::load_folder_covers,
            commands::library::get_progress_all,
            commands::library::set_progress,
            commands::library::analyze_loudness,
            commands::library::cancel_loudness,
            commands::library::mark_track_completed,
            commands::library::clear_track_progress,
            commands::library::list_bookmarks,
            commands::library::add_bookmark,
            commands::library::rename_bookmark,
            commands::library::remove_bookmark,
            commands::library::start_watching,
            commands::library::stop_watching,
            commands::system::load_settings,
            commands::system::save_settings,
            commands::system::load_bg_image,
            commands::system::save_bg_image,
            commands::system::remove_bg_image,
            commands::system::startup_warnings,
            commands::system::minimize_window,
            commands::system::maximize_window,
            commands::system::close_window,
            commands::system::is_maximized,
            commands::system::force_close_window,
            commands::system::toggle_fullscreen,
            commands::system::ffmpeg_exec,
            commands::system::ffmpeg_probe,
            commands::system::convert_run,
            commands::system::convert_cancel,
            commands::system::ai_chat,
            commands::system::ai_ping,
            commands::system::ai_list_models,
            commands::player::player_play,
            commands::player::player_toggle,
            commands::player::player_seek,
            commands::player::player_set_volume,
            commands::player::player_set_muted,
            commands::player::player_set_speed,
            commands::player::player_stop,
            commands::player::player_set_video_enabled,
            commands::player::player_set_stage_rect,
            commands::player::player_get_state,
            commands::player::player_subtitle_tracks,
            commands::player::player_set_subtitle_track,
            commands::player::player_set_subtitle_visible,
            commands::player::player_set_subtitle_delay,
            commands::player::player_set_sub_style,
            commands::player::player_apply_loudness_gain,
            commands::player::player_list_devices,
            commands::player::player_set_device,
            commands::player::pip_open,
            commands::player::pip_ready,
            commands::player::pip_close,
            commands::player::pip_restore,
            commands::player::pip_set_pinned,
            commands::player::app_force_quit,
            commands::library::taxonomy_list_categories,
            commands::library::taxonomy_create_category,
            commands::library::taxonomy_rename_category,
            commands::library::taxonomy_delete_category,
            commands::library::taxonomy_move_category,
            commands::library::taxonomy_assign_category,
            commands::library::taxonomy_unassign_category,
            commands::library::taxonomy_list_tags,
            commands::library::taxonomy_upsert_tag,
            commands::library::taxonomy_rename_tag,
            commands::library::taxonomy_delete_tag,
            commands::library::taxonomy_tag_tracks,
            commands::library::taxonomy_untag_tracks,
            commands::library::taxonomy_suggest_tags,
            commands::library::taxonomy_apply_suggested_tags,
            commands::library::taxonomy_category_track_ids,
            commands::library::taxonomy_track_tags,
            commands::library::taxonomy_all_track_tags,
            commands::library::taxonomy_all_category_items,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| {
            crate::paths::append_log(&format!("fatal: run exited: {e}"));
            Box::new(e) as Box<dyn std::error::Error>
        })
}

fn build_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("KX 音乐播放器");
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "show" => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }
        "quit" => {
            // 与 app_force_quit 同语义：先置 force_close（否则主窗口 CloseRequested
            // 会被 prevent 成“隐藏到托盘”），再广播 beforeClose 给前端落盘设置，
            // 300ms 后退出；原实现先调 app.exit(0) 再 sleep，保存与退出互相竞态。
            let state: tauri::State<AppWindowsState> = app.state();
            state.force_close.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = app.emit("window:beforeClose", ());
            let app2 = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                crate::fftools::kill_all();
                app2.exit(0);
            });
        }
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, .. } = event {
            let app = tray.app_handle();
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }
    })
    .build(app)?;
    Ok(())
}
