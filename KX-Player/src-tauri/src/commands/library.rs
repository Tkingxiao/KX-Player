//! 命令层：曲库/扫描/封面/进度/书签/文件监听。
//! 全部 async + spawn_blocking：Tauri 同步命令会在主线程执行，任何 IO 都可能卡住 UI。

use crate::model::{Bookmark, PlayProgress, ScanResult};
use std::path::PathBuf;
use tauri::{AppHandle, State};

use crate::state::AppWindowsState;

fn db_path() -> PathBuf {
    crate::paths::library_db_path()
}

#[tauri::command]
pub async fn scan_folders(app: AppHandle, paths: Vec<String>) -> Option<ScanResult> {
    if paths.is_empty() {
        return None;
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Some(crate::scanner::scan_full(&app2, &paths, &db_path()))
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub async fn scan_folders_incremental(app: AppHandle, paths: Vec<String>) -> Option<ScanResult> {
    if paths.is_empty() {
        return None;
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Some(crate::scanner::scan_incremental(&app2, &paths, &db_path()))
    })
    .await
    .unwrap_or(None)
}

/// 启动用静默增量同步：无变更时不发 stage/progress 事件，避免 UI 闪出「构建音乐库」与转圈。
#[tauri::command]
pub async fn startup_sync(app: AppHandle, paths: Vec<String>) -> Option<ScanResult> {
    if paths.is_empty() {
        return None;
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Some(crate::scanner::scan_incremental_quiet(&app2, &paths, &db_path()).0)
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub async fn remove_folder(app: AppHandle, folder_path: String, remaining_paths: Vec<String>) -> Option<ScanResult> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Some(crate::scanner::remove_folder(&app2, &folder_path, &remaining_paths, &db_path()))
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub async fn load_library() -> Option<ScanResult> {
    tauri::async_runtime::spawn_blocking(|| crate::db::library::load_snapshot(&db_path(), false))
        .await
        .unwrap_or(None)
}

#[tauri::command]
pub async fn load_library_fast() -> Option<ScanResult> {
    tauri::async_runtime::spawn_blocking(|| crate::db::library::load_snapshot(&db_path(), true))
        .await
        .unwrap_or(None)
}

#[tauri::command]
pub async fn get_track_covers(track_ids: Vec<String>) -> std::collections::HashMap<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = std::collections::HashMap::new();
        for id in track_ids {
            if let Some(p) = crate::scanner::covers::track_cover_path(&id) {
                out.insert(id, p);
            }
        }
        out
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn get_folder_covers(folder_paths: Vec<String>) -> std::collections::HashMap<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = std::collections::HashMap::new();
        for p in folder_paths {
            // 先查映射表；未命中时按需扫描目录并生成封面（首次较慢，之后走缓存）
            if let Some(c) = crate::scanner::covers::resolve_folder_cover(&p) {
                out.insert(p, c);
            }
        }
        out
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn load_folder_covers() -> std::collections::HashMap<String, String> {
    tauri::async_runtime::spawn_blocking(|| crate::scanner::covers::all_folder_cover_paths().into_iter().collect())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn get_progress_all() -> Vec<PlayProgress> {
    tauri::async_runtime::spawn_blocking(|| crate::db::progress::get_all_progress(&db_path()))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn set_progress(track_id: String, position_ms: i64, completed: Option<bool>, last_speed: Option<f64>) -> bool {
    if track_id.is_empty() || position_ms < 0 {
        return false;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::set_progress(&db_path(), &track_id, position_ms, completed, last_speed);
    })
    .await
    .is_ok()
}

/// 响度分析任务代际旗标：每次启动新任务时 bump 代际并复位取消位；
/// 旧线程通过对比代际知道自己已被取代（替代全局互斥，逻辑简单无死锁）。
static LOUDNESS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LOUDNESS_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 后台响度分析：逐文件 ffmpeg ebur128，进度走事件，结果写回 DB。
/// 任务 + 事件模式（红线条款：长任务不得同步阻塞 IPC）。
/// 事件 `loudness:progress`：{ completed, total, current, done, failed, finished, cancelled }
#[tauri::command]
pub fn analyze_loudness(app: AppHandle, tracks: Vec<(String, String)>) -> bool {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    if tracks.is_empty() {
        return false;
    }
    let gen = LOUDNESS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    LOUDNESS_CANCEL.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    let total = tracks.len();
    std::thread::spawn(move || {
        let mut completed = 0usize;
        let mut done = 0usize;
        let mut failed = 0usize;
        let mut cancelled = false;
        for (id, path) in &tracks {
            // 被新任务取代或收到取消请求时退出
            if LOUDNESS_CANCEL.load(Ordering::SeqCst) || LOUDNESS_GENERATION.load(Ordering::SeqCst) != gen {
                cancelled = true;
                break;
            }
            let result = crate::fftools::measure_loudness(path);
            completed += 1;
            match result.lufs {
                Some(v) => {
                    done += 1;
                    let _ = crate::db::library::set_track_loudness(&db_path(), id, v);
                }
                None => {
                    failed += 1;
                    crate::paths::append_log(&format!(
                        "[loudness] {} failed: {}",
                        path,
                        result.error.unwrap_or_default()
                    ));
                }
            }
            let _ = app2.emit(
                "loudness:progress",
                serde_json::json!({
                    "completed": completed,
                    "total": total,
                    "current": path,
                    "done": done,
                    "failed": failed,
                    "finished": false,
                    "cancelled": false,
                }),
            );
        }
        // 仅当仍是当前任务时才发完成帧，避免旧任务覆盖新任务状态
        if LOUDNESS_GENERATION.load(Ordering::SeqCst) == gen {
            let _ = app2.emit(
                "loudness:progress",
                serde_json::json!({
                    "completed": completed,
                    "total": total,
                    "current": "",
                    "done": done,
                    "failed": failed,
                    "finished": true,
                    "cancelled": cancelled,
                }),
            );
        }
    });
    true
}

/// 取消正在进行的响度分析（幂等）
#[tauri::command]
pub fn cancel_loudness() -> bool {
    use std::sync::atomic::Ordering;
    LOUDNESS_CANCEL.store(true, Ordering::SeqCst);
    true
}

#[tauri::command]
pub async fn mark_track_completed(track_id: String, completed: bool) -> bool {
    if track_id.is_empty() {
        return false;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::mark_completed(&db_path(), &track_id, completed);
    })
    .await
    .is_ok()
}

#[tauri::command]
pub async fn clear_track_progress(track_id: String) -> bool {
    if track_id.is_empty() {
        return false;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::clear_progress(&db_path(), &track_id);
    })
    .await
    .is_ok()
}

#[tauri::command]
pub async fn list_bookmarks(track_id: String) -> Vec<Bookmark> {
    if track_id.is_empty() {
        return vec![];
    }
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::list_bookmarks(&db_path(), &track_id))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn add_bookmark(track_id: String, at_ms: i64, label: String) -> Option<Bookmark> {
    if track_id.is_empty() || at_ms < 0 {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::add_bookmark(&db_path(), &track_id, at_ms, &label))
        .await
        .unwrap_or(None)
}

#[tauri::command]
pub async fn rename_bookmark(id: String, label: String) -> bool {
    if id.is_empty() {
        return false;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::rename_bookmark(&db_path(), &id, &label);
    })
    .await
    .is_ok()
}

#[tauri::command]
pub async fn remove_bookmark(id: String) -> bool {
    if id.is_empty() {
        return false;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::remove_bookmark(&db_path(), &id);
    })
    .await
    .is_ok()
}

#[tauri::command]
pub fn start_watching(paths: Vec<String>, state: State<'_, AppWindowsState>) -> bool {
    crate::watcher::start(&paths, state.watcher.clone());
    true
}

#[tauri::command]
pub fn stop_watching(state: State<'_, AppWindowsState>) -> bool {
    crate::watcher::stop(&state.watcher);
    true
}

// ── 分类与标签（同样全部走阻塞池）──────────────────────────────

macro_rules! blocking {
    ($expr:expr) => {
        tauri::async_runtime::spawn_blocking(move || $expr).await
    };
}

#[tauri::command]
pub async fn taxonomy_list_categories() -> Vec<crate::taxonomy::CategoryDto> {
    blocking!(crate::taxonomy::build_category_tree(crate::taxonomy::list_categories())).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_create_category(parent_id: Option<i64>, name: String) -> Result<crate::taxonomy::CategoryDto, String> {
    let trimmed = name.trim().to_string();
    let id = blocking!(crate::taxonomy::create_category(parent_id, &name))
        .map_err(|_| "任务失败".to_string())?
        ?;
    Ok(crate::taxonomy::CategoryDto {
        id,
        parent_id,
        name: trimmed,
        icon: None,
        color: None,
        sort_index: 0,
        track_count: 0,
        children: vec![],
    })
}

#[tauri::command]
pub async fn taxonomy_rename_category(id: i64, name: String) -> Result<(), String> {
    blocking!(crate::taxonomy::rename_category(id, &name)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_delete_category(id: i64) -> Result<(), String> {
    blocking!(crate::taxonomy::delete_category(id)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_move_category(id: i64, parent_id: Option<i64>, sort_index: i64) -> Result<(), String> {
    blocking!(crate::taxonomy::move_category(id, parent_id, sort_index)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_assign_category(category_id: i64, track_ids: Vec<String>) -> Result<i64, String> {
    blocking!(crate::taxonomy::assign_category(category_id, &track_ids)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_unassign_category(category_id: i64, track_ids: Vec<String>) -> Result<i64, String> {
    blocking!(crate::taxonomy::unassign_category(category_id, &track_ids)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_list_tags() -> Vec<crate::taxonomy::TagDto> {
    blocking!(crate::taxonomy::list_tags()).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_upsert_tag(name: String, color: Option<String>, kind: Option<String>) -> Result<crate::taxonomy::TagDto, String> {
    let trimmed = name.trim().to_string();
    let kind_final = kind.unwrap_or_else(|| "topic".into());
    let color2 = color.clone();
    let kind2 = kind_final.clone();
    let id = blocking!(crate::taxonomy::upsert_tag(&name, color2.as_deref(), &kind2))
        .map_err(|_| "任务失败".to_string())?
        ?;
    Ok(crate::taxonomy::TagDto {
        id,
        name: trimmed,
        color,
        kind: kind_final,
        use_count: 0,
        auto_generated: false,
    })
}

#[tauri::command]
pub async fn taxonomy_rename_tag(id: i64, name: String) -> Result<(), String> {
    blocking!(crate::taxonomy::rename_tag(id, &name)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_delete_tag(id: i64) -> Result<(), String> {
    blocking!(crate::taxonomy::delete_tag(id)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_tag_tracks(tag_ids: Vec<i64>, track_ids: Vec<String>, mode: Option<String>) -> Result<i64, String> {
    blocking!(crate::taxonomy::tag_tracks(&tag_ids, &track_ids, mode.as_deref().unwrap_or("add")))
        .map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_untag_tracks(tag_id: i64, track_ids: Vec<String>) -> Result<i64, String> {
    blocking!(crate::taxonomy::untag_tracks(tag_id, &track_ids)).map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_suggest_tags(track_ids: Vec<String>) -> Vec<(String, Vec<crate::taxonomy::TagSuggestionDto>)> {
    blocking!(crate::taxonomy::suggest_tags(&track_ids)).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_apply_suggested_tags(accepted: Vec<(String, String)>) -> Result<i64, String> {
    // accepted: (trackId, tagName) — 先建 tag 再打标
    tauri::async_runtime::spawn_blocking(move || -> Result<i64, String> {
        let mut tag_ids: Vec<i64> = Vec::new();
        let names: Vec<String> = accepted.iter().map(|(_, n)| n.clone()).collect();
        for name in &names {
            let id = crate::taxonomy::upsert_tag(name, None, "topic")?;
            if !tag_ids.contains(&id) {
                tag_ids.push(id);
            }
        }
        let mut track_ids: Vec<String> = accepted.iter().map(|(t, _)| t.clone()).collect();
        track_ids.dedup();
        crate::taxonomy::tag_tracks(&tag_ids, &track_ids, "add")
    })
    .await
    .map_err(|_| "任务失败".to_string())?
}

#[tauri::command]
pub async fn taxonomy_category_track_ids(category_id: i64, include_children: Option<bool>) -> Vec<String> {
    blocking!(crate::taxonomy::category_track_ids(category_id, include_children.unwrap_or(true))).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_track_tags(track_ids: Vec<String>) -> std::collections::HashMap<String, Vec<crate::taxonomy::TagDto>> {
    blocking!(crate::taxonomy::tags_of_tracks(&track_ids)).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_all_track_tags() -> Vec<(String, i64)> {
    blocking!(crate::taxonomy::all_track_tags()).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_all_category_items() -> Vec<(i64, String)> {
    blocking!(crate::taxonomy::all_category_items()).unwrap_or_default()
}
