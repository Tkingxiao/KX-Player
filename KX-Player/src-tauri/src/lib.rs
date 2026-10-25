//! 应用装配：插件/托盘/关窗到托盘/单实例/命令注册。

mod ai;
mod ai_batch;
mod ai_cache;
mod ai_chunk;
mod ai_event;
mod ai_export;
mod ai_file;
mod ai_job;
mod ai_prompt;
mod ai_scan;
mod ai_strategy;
mod ai_translated;
mod ai_url;
mod ai_wire;
mod atomic;
mod bgimage;
mod commands;
mod db;
mod error;
mod ffprobe;
mod fftools;
mod journal;
mod limits;
mod loudness;
mod model;
mod paths;
mod player;
mod rename;
mod scanner;
mod scope;
mod settingsio;
mod shutdown;
mod state;
mod subtitles;
mod taxonomy;
mod watcher;

use state::AppWindowsState;
use tauri::tray::TrayIconBuilder;
use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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
            // 数据目录 = 安装目录旁的 `data/`。先建目录、再搬老位置的数据，顺序不能反：
            // 搬迁的判据是「新目录还是空的」，先建封面目录就把这条判据污染了。
            let data_usable = paths::ensure_data_dir().is_ok();
            let moved = paths::migrate_legacy_data();
            let _ = paths::ensure_covers_dir();
            if !data_usable {
                state::startup_warn(format!(
                    "安装目录不可写，曲库与设置存不下来：请把应用装到一个可写的目录里（数据目录 {}）",
                    paths::data_dir().display()
                ));
            }
            if moved > 0 {
                paths::append_log(&format!(
                    "[迁移] 老数据目录的 {moved} 项已搬进安装目录 {}",
                    paths::data_dir().display()
                ));
            }
            db::check_integrity();
            // P1-11：改名与旧 ID 迁移都跨「文件系统 + 曲库 + 设置」三处，崩在中间留下的是一半
            // 改完的盘和一本没写完的账。启动时先把没删掉的意图核对一遍：改名只补账、不动盘；
            // 旧 ID 迁移的封面该搬的补搬（判据只有「旧名在、新名不在」）。
            rename::journal::recover_at_startup();
            let migrated = db::migrate_legacy_ids(&paths::library_db_path(), &paths::settings_path());
            if migrated > 0 {
                paths::append_log(&format!("条目 ID 迁移：{migrated} 条旧 ID 已改写为 UUIDv5，备份见 *.pre-uuid5"));
            }
            if let Some(line) = db::recover_legacy_ids(&paths::settings_path()) {
                paths::append_log(&format!("[迁移] 崩溃后恢复：{line}"));
                state::startup_warn(format!(
                    "上次的旧 ID 迁移没有正常结束：{line}。已按现状补完，若封面显示异常可在设置中重新扫描。"
                ));
            }
            scanner::covers::load_folder_cover_map();
            bgimage::migrate_oversized();
            // P1-12：把「这次用的是哪个 ffmpeg」记进日志（后台线程，不挡启动）
            fftools::log_ffmpeg_identity();

            // asset 协议范围：封面目录（含子目录）+ 背景图这一个文件。
            // 原来整个数据目录都放开了，等于把曲库 SQLite、设置与日志一并交给 WebView 去 fetch ——
            // 它们各自有命令可读，不需要再开一条不经过命令层的口子。
            let asset = app.asset_protocol_scope();
            let _ = asset.allow_directory(paths::covers_dir(), true);
            let _ = asset.allow_file(paths::bg_image_path());

            // P0-1 路径能力：授权的三个来路在这里登记两个，第三个是原生对话框（commands::dialog_fs）
            let scope = scope::PathScope::new();
            scope.allow_root(&paths::covers_dir());
            scope.allow_root(&paths::data_dir());
            for root in db::library::folder_paths(&paths::library_db_path()) {
                scope.allow_root(std::path::Path::new(&root));
            }
            // 第四条来路：翻译账里记录过的目录（应用写入的账，是用户当年在对话框里授权过的延续）。
            // 没有它，重启后译文管理页的「打开译文 / 重译」全被闸门拒掉——点击无声无息。
            for dir in db::with_db(&paths::library_db_path(), |c| Ok(ai_translated::ledger_dirs(c))).unwrap_or_default() {
                scope.allow_root(&dir);
            }
            app.manage(scope);

            state::set_app_handle(app.handle().clone());
            app.manage(AppWindowsState::new());
            app.manage(PlayerCore::new(app.handle().clone()));

            // 主窗口在代码里建（配置里那条已删，`app.windows` 留空）。原因是 WebView 的
            // 数据文件夹（localStorage / Cookie / 缓存）：配置项 `dataDirectory` 只能写
            // 「相对 appDataDir」的路径，写不出安装目录，而它一缺位 Tauri 就把这份数据
            // 放回 `%LOCALAPPDATA%/<identifier>`。悬浮窗用同一份，见 `commands::player`。
            let mut main_win = WebviewWindowBuilder::new(
                app.handle(),
                "main",
                WebviewUrl::App("index.html".into()),
            )
            .title("KX Player")
            .inner_size(1200.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .decorations(false)
            .transparent(true)
            .center();
            if let Some(dir) = paths::webview_data_dir_if_usable() {
                main_win = main_win.data_directory(dir);
            }
            main_win.build()?;

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
                // 往窗口里拖文件 = 用户手势，与「对话框选中」同级：在这里登记授权，
                // 前端随后递回来的同一批路径才过得了 [PathScope]（不靠前端自报授权）。
                WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    let scope: tauri::State<scope::PathScope> = window.app_handle().state();
                    for p in paths {
                        let _ = scope.allow_file(p);
                    }
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
            commands::dialog_fs::open_media_files,
            commands::dialog_fs::open_text_files,
            commands::dialog_fs::select_bg_image,
            commands::dialog_fs::read_as_data_url,
            commands::dialog_fs::read_text_file,
            commands::dialog_fs::file_exists,
            commands::dialog_fs::list_dir,
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
            commands::system::shutdown_ready,
            commands::system::toggle_fullscreen,
            commands::system::ffmpeg_probe,
            commands::system::convert_run,
            commands::system::convert_cancel,
            commands::system::ai_chat,
            commands::system::ai_ping,
            commands::system::ai_list_models,
            commands::system::ai_endpoint_check,
            commands::system::ai_plaintext_set,
            commands::system::ai_translated_list,
            commands::system::ai_translated_cleanup,
            commands::system::ai_translated_remove,
            commands::system::ai_translated_remove_source,
            commands::system::ai_translate_start,
            commands::system::ai_texts_start,
            commands::system::ai_task_cancel,
            commands::system::ai_export_prompts,
            commands::system::ai_import_translations,
            commands::system::subtitle_scan_dir,
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
            commands::rename::rename_preview,
            commands::rename::rename_suggest_names,
            commands::rename::rename_reject_suggestion,
            commands::rename::rename_apply,
            commands::rename::rename_rollback,
            commands::rename::rename_redo,
            commands::rename::rename_batches,
            commands::taxonomy::taxonomy_list_categories,
            commands::taxonomy::taxonomy_create_category,
            commands::taxonomy::taxonomy_rename_category,
            commands::taxonomy::taxonomy_delete_category,
            commands::taxonomy::taxonomy_move_category,
            commands::taxonomy::taxonomy_assign_category,
            commands::taxonomy::taxonomy_unassign_category,
            commands::taxonomy::taxonomy_list_tags,
            commands::taxonomy::taxonomy_upsert_tag,
            commands::taxonomy::taxonomy_rename_tag,
            commands::taxonomy::taxonomy_delete_tag,
            commands::taxonomy::taxonomy_tag_tracks,
            commands::taxonomy::taxonomy_untag_tracks,
            commands::taxonomy::taxonomy_suggest_tags,
            commands::taxonomy::taxonomy_apply_suggested_tags,
            commands::taxonomy::taxonomy_category_track_ids,
            commands::taxonomy::taxonomy_track_tags,
            commands::taxonomy::taxonomy_all_track_tags,
            commands::taxonomy::taxonomy_all_category_items,
        ])
        .build(tauri::generate_context!())
        .map_err(|e| {
            crate::paths::append_log(&format!("fatal: build failed: {e}"));
            Box::new(e) as Box<dyn std::error::Error>
        })?
        .run(|app, event| {
            // 退出的唯一入口（P1-3）：窗口全关、系统会话结束、程序化 exit 都会走到这里，
            // 先拦下来交给 supervisor；它跑完自己调 `app.exit(0)` —— 那时 `exiting()`
            // 已是 true，这一次请求放行，进程才真的结束。
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !shutdown::exiting() {
                    api.prevent_exit();
                    shutdown::request_exit(app, "系统请求退出");
                }
            }
        });
    Ok(())
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
            // 退出只有一条路（P1-3）：收尾的次序、预算与记账都在 `shutdown.rs`
            // （停止接收 → 取消 → 停 watcher → 等任务 → kill+wait → 等落盘回执 → 收线程 → exit）。
            shutdown::request_exit(app, "托盘退出");
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
