//! 命令层：曲库/扫描/封面/进度/书签/文件监听。
//! 全部 async + spawn_blocking：Tauri 同步命令会在主线程执行，任何 IO 都可能卡住 UI。

use crate::error::{IpcError, IpcResult};
use crate::model::{Bookmark, PlayProgress, ScanResult};
use crate::scope::PathScope;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

use crate::state::AppWindowsState;

fn db_path() -> PathBuf {
    crate::paths::library_db_path()
}

/// 扫描/改名这类「一批路径」入口的统一闸门（P0-1）：整批都得在授权范围内。
/// 授权来自对话框选中的文件夹与库里已登记的根目录 —— 前端换一条路径递进来，这里就把它拦下。
fn guard_roots(app: &AppHandle, paths: &[String], cmd: &str) -> bool {
    match app.state::<PathScope>().check_all(paths) {
        Ok(()) => true,
        Err((bad, why)) => {
            crate::paths::append_log(&format!("[scope] {cmd} 拒绝整批：{bad}（{why:?}）"));
            false
        }
    }
}

/// 退出途中不再开扫描（P1-3 第一步「停止接收」）：回一份 `cancelled` 的现有快照，
/// 前端据此保留曲库 —— 而不是等一个注定被退出打断的结果。
fn refuse_scan_while_exiting(paths: &[String]) -> Option<ScanResult> {
    if crate::shutdown::accepting() {
        return None;
    }
    crate::paths::append_log("[shutdown] 已在收尾，拒绝开始新的扫描");
    Some(crate::scanner::cancelled_for_exit(&db_path(), paths))
}

#[tauri::command]
pub async fn scan_folders(app: AppHandle, paths: Vec<String>) -> Option<ScanResult> {
    if paths.is_empty() || !guard_roots(&app, &paths, "scan_folders") {
        return None;
    }
    if let Some(refused) = refuse_scan_while_exiting(&paths) {
        return Some(refused);
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
    if paths.is_empty() || !guard_roots(&app, &paths, "scan_folders_incremental") {
        return None;
    }
    if let Some(refused) = refuse_scan_while_exiting(&paths) {
        return Some(refused);
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
    if paths.is_empty() || !guard_roots(&app, &paths, "startup_sync") {
        return None;
    }
    if let Some(refused) = refuse_scan_while_exiting(&paths) {
        return Some(refused);
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
    // 被移除的这个根与留下来的那些都得是授权的：前者用来确认「确实是从曲库里摘的」，
    // 后者是下一次扫描的输入（扫描本身也拦一道，这里是让失败更早、更明确）
    if !guard_roots(&app, std::slice::from_ref(&folder_path), "remove_folder") || !guard_roots(&app, &remaining_paths, "remove_folder") {
        return None;
    }
    let app2 = app.clone();
    let removed = folder_path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        Some(crate::scanner::remove_folder(&app2, &folder_path, &remaining_paths, &db_path()))
    })
    .await
    .unwrap_or(None);
    // 从库里摘掉的目录当场收权：留着的话，被移除的根在本次会话里仍然是可读的
    let _ = app.state::<PathScope>().revoke_root(Path::new(&removed));
    result
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
pub async fn get_folder_covers(app: AppHandle, folder_paths: Vec<String>) -> std::collections::HashMap<String, String> {
    // 逐条筛：这个命令会**按参数指定的目录**去找封面图，路径必须落在授权根里。
    // 这里不整批拒 —— 一条离线盘上的目录不该让整屏封面都不出来（扫描/改名那两处是整批拒，
    // 因为它们是「要么全做要么不做」的语义）。
    let scope = app.state::<PathScope>();
    let folder_paths: Vec<String> = folder_paths
        .into_iter()
        .filter(|p| {
            let ok = scope.allows(p);
            if !ok {
                crate::paths::append_log(&format!("[scope] get_folder_covers 跳过 {p}（Outside）"));
            }
            ok
        })
        .collect();
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
pub async fn get_progress_all() -> IpcResult<Vec<PlayProgress>> {
    tauri::async_runtime::spawn_blocking(|| crate::db::progress::get_all_progress(&db_path()))
        .await
        .map_err(|e| IpcError::join_failed("读播放进度", e))?
        .map_err(IpcError::db)
}

#[tauri::command]
pub async fn set_progress(track_id: String, position_ms: i64, completed: Option<bool>, last_speed: Option<f64>) -> IpcResult<bool> {
    if track_id.is_empty() || position_ms < 0 {
        return Err(IpcError::invalid_argument("曲目 ID 不能为空，进度不能为负"));
    }
    let db = db_path();
    tauri::async_runtime::spawn_blocking(move || {
        crate::db::progress::set_progress(&db, &track_id, position_ms, completed, last_speed)
    })
    .await
    .map_err(|e| IpcError::join_failed("写播放进度", e))?
    .map_err(IpcError::db)?;
    Ok(true)
}

/// 响度分析任务代际旗标：每次启动新任务时 bump 代际并复位取消位；
/// 旧线程通过对比代际知道自己已被取代（替代全局互斥，逻辑简单无死锁）。
static LOUDNESS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LOUDNESS_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 响度任务是否在跑（退出收尾等它归零的判据）：线程起来就置位，走到最后一行才清。
static LOUDNESS_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 在跑吗（`shutdown.rs` 的「等任务收尾」用）。
pub(crate) fn loudness_running() -> bool {
    LOUDNESS_RUNNING.load(std::sync::atomic::Ordering::SeqCst)
}

/// 请求取消（退出收尾与 `cancel_loudness` 命令共用一处实现）；返回「此刻是否真的有任务在跑」。
pub(crate) fn request_cancel_loudness() -> bool {
    LOUDNESS_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
    loudness_running()
}

/// 并发度：封顶 4 路。再多没有意义 —— 每一路都是一个 ffmpeg 在解整轨，瓶颈是磁盘
/// 顺序读和解码线程，而不是任务数；同时给实时播放留出核心（P1-9 的同一条理由）。
fn loudness_workers() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    cores.clamp(1, 4)
}

/// 一批任务的共用状态。`no_ffmpeg` 是批内的「到此为止」：二进制缺失时每条都会失败在
/// 同一个原因上，第一例记完日志就不必再空转剩下的几百条。
struct LoudnessBatch {
    gen: u64,
    no_ffmpeg: std::sync::atomic::AtomicBool,
}

/// 这一批还要不要继续：被新任务取代、用户取消、应用正在退出收尾、或已确认没有 ffmpeg。
fn loudness_stopped(batch: &LoudnessBatch) -> bool {
    use std::sync::atomic::Ordering;
    batch.no_ffmpeg.load(Ordering::SeqCst)
        || LOUDNESS_CANCEL.load(Ordering::SeqCst)
        || LOUDNESS_GENERATION.load(Ordering::SeqCst) != batch.gen
        || crate::fftools::shutdown_requested()
}

/// 一条测量结果的去向。`Silent` 与 `Failed` 分家：数字静音是**测准了**（只是没有响度），
/// 「没测成」才是失败 —— 旧实现把两者一起记进 failed。
enum Outcome {
    Value(f64),
    Silent,
    Failed(String),
    /// 没起 ffmpeg 就收手了
    Skipped,
}

struct Measured {
    id: String,
    path: String,
    outcome: Outcome,
}

/// 后台响度分析：ffmpeg 只解码出 PCM，积分响度在 Rust 侧算（见 `loudness.rs`）。
/// 任务 + 事件模式（红线条款：长任务不得同步阻塞 IPC）。
/// 事件 `loudness:progress`：{ completed, total, current, done, silent, failed, finished, cancelled }
#[tauri::command]
pub fn analyze_loudness(app: AppHandle, scope: State<'_, PathScope>, tracks: Vec<(String, String)>) -> bool {
    use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    // 退出途中不再开新队列（P1-3 第一步「停止接收」）
    if !crate::shutdown::accepting() {
        return false;
    }
    // 逐条筛（P0-1）：这条链路会拿参数里的路径去喂 ffmpeg 读文件。离线盘上的条目会被
    // `resolve` 判不可解析 —— 与「路径本来就读不到」是同一个结局，不该因此拖垮整批
    let tracks: Vec<(String, String)> = tracks
        .into_iter()
        .filter(|(_, path)| {
            let ok = scope.resolve(path).is_ok();
            if !ok {
                crate::paths::append_log(&format!("[scope] analyze_loudness 跳过 {path}"));
            }
            ok
        })
        .collect();
    if tracks.is_empty() {
        return false;
    }
    let gen = LOUDNESS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    LOUDNESS_CANCEL.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    let total = tracks.len();
    std::thread::spawn(move || {
        LOUDNESS_RUNNING.store(true, Ordering::SeqCst);
        let batch = std::sync::Arc::new(LoudnessBatch {
            gen,
            no_ffmpeg: std::sync::atomic::AtomicBool::new(false),
        });
        let workers = loudness_workers();
        // 有界：攒够 2×并发度就堵在生产方上，内存与任务数无关
        let (tx, rx) = crossbeam_channel::bounded::<Measured>(workers * 2);

        // 写库 + 发进度收在**一条**线程上：SQLite 没设 busy_timeout，几个 writer 并发
        // 就会撞 SQLITE_BUSY（丢的是已经算出来的响度值）；顺带让计数按完成顺序单调走。
        let emitter = app2.clone();
        let consumer = std::thread::spawn(move || {
            let (mut completed, mut done, mut silent, mut failed, mut skipped) = (0usize, 0, 0, 0, 0);
            for m in rx.iter() {
                match &m.outcome {
                    // 没起 ffmpeg 就收手的（取消 / 被取代 / 退出收尾）不进计数也不发帧：
                    // 一旦收手，剩下的条目会一条接一条地 Skipped，发出去只是刷 WebView
                    Outcome::Skipped => {
                        skipped += 1;
                        continue;
                    }
                    Outcome::Value(v) => {
                        // 量出来了但写库失败 = 这条对用户仍是「没结果」，进 failed 计数而不是无声丢弃 ——
                        // 要不然设置页的「有响度」数会凭空比实际多（P1-6）。
                        match crate::db::library::set_track_loudness(&db_path(), &m.id, *v) {
                            Ok(_) => done += 1,
                            Err(e) => {
                                failed += 1;
                                crate::paths::append_log(&format!("[loudness] {} 响度写库失败：{e}", m.path));
                            }
                        }
                    }
                    Outcome::Silent => {
                        silent += 1;
                        crate::paths::append_log(&format!("[loudness] {} 无有效响度（数字静音）", m.path));
                    }
                    Outcome::Failed(e) => {
                        failed += 1;
                        crate::paths::append_log(&format!("[loudness] {} failed: {e}", m.path));
                    }
                }
                completed += 1;
                if let Err(e) = emitter.emit(
                    "loudness:progress",
                    serde_json::json!({
                        "completed": completed,
                        "total": total,
                        "current": m.path,
                        "done": done,
                        "silent": silent,
                        "failed": failed,
                        "finished": false,
                        "cancelled": false,
                    }),
                ) {
                    // WebView 已关（退出途中）或接收端出错：这一帧丢了，账上记一笔再走
                    crate::paths::append_log(&format!("[loudness] 进度事件发送失败（{}）：{e}", m.path));
                }
            }
            (completed, done, silent, failed, skipped)
        });

        // 只有解码这一段并行；`measure_one` 每次开工前先认一次代际/取消
        let run = |tx: crossbeam_channel::Sender<Measured>| match rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
        {
            Ok(pool) => pool.install(|| {
                tracks.par_iter().for_each_with(tx.clone(), |s, (id, path)| {
                    let _ = s.send(measure_one(&batch, id, path));
                });
            }),
            Err(e) => {
                crate::paths::append_log(&format!("[loudness] 线程池建不起来（{e}），退成逐个测量"));
                for (id, path) in &tracks {
                    if tx.send(measure_one(&batch, id, path)).is_err() {
                        break;
                    }
                }
            }
        };
        run(tx.clone());
        // 生产方全部收手：放下最后一个发送端，消费者读到关闭就退出
        drop(tx);

        let (completed, done, silent, failed, skipped) = consumer.join().unwrap_or((0, 0, 0, 0, 0));
        // 仅当仍是当前任务时才发完成帧，避免旧任务覆盖新任务状态
        if LOUDNESS_GENERATION.load(Ordering::SeqCst) == gen {
            if let Err(e) = app2.emit(
                "loudness:progress",
                serde_json::json!({
                    "completed": completed,
                    "total": total,
                    "current": "",
                    "done": done,
                    "silent": silent,
                    "failed": failed,
                    "finished": true,
                    "cancelled": skipped > 0,
                }),
            ) {
                crate::paths::append_log(&format!("[loudness] 完成帧发送失败：{e}"));
            }
        }
        LOUDNESS_RUNNING.store(false, Ordering::SeqCst);
    });
    true
}

/// 测一条：先认旗标，再起 ffmpeg。ffmpeg 缺失时给批内共享位置一次，整批就此收手。
fn measure_one(batch: &LoudnessBatch, id: &str, path: &str) -> Measured {
    use std::sync::atomic::Ordering;
    let outcome = if loudness_stopped(batch) {
        Outcome::Skipped
    } else {
        match crate::loudness::measure_loudness(Path::new(path)) {
            Ok(Some(v)) => Outcome::Value(v),
            Ok(None) => Outcome::Silent,
            Err(crate::loudness::LoudnessError::FfmpegMissing) => {
                batch.no_ffmpeg.store(true, Ordering::SeqCst);
                Outcome::Failed("ffmpeg.exe 未找到".into())
            }
            Err(e) => Outcome::Failed(e.to_string()),
        }
    };
    Measured { id: id.to_string(), path: path.to_string(), outcome }
}

/// 取消正在进行的响度分析（幂等）
#[tauri::command]
pub fn cancel_loudness() -> bool {
    request_cancel_loudness()
}

#[tauri::command]
pub async fn mark_track_completed(track_id: String, completed: bool) -> IpcResult<bool> {
    if track_id.is_empty() {
        return Err(IpcError::invalid_argument("曲目 ID 不能为空"));
    }
    let db = db_path();
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::mark_completed(&db, &track_id, completed))
        .await
        .map_err(|e| IpcError::join_failed("写完成标记", e))?
        .map_err(IpcError::db)?;
    Ok(true)
}

#[tauri::command]
pub async fn clear_track_progress(track_id: String) -> IpcResult<bool> {
    if track_id.is_empty() {
        return Err(IpcError::invalid_argument("曲目 ID 不能为空"));
    }
    let db = db_path();
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::clear_progress(&db, &track_id))
        .await
        .map_err(|e| IpcError::join_failed("清播放进度", e))?
        .map_err(IpcError::db)?;
    Ok(true)
}

#[tauri::command]
pub async fn list_bookmarks(track_id: String) -> IpcResult<Vec<Bookmark>> {
    if track_id.is_empty() {
        return Err(IpcError::invalid_argument("曲目 ID 不能为空"));
    }
    let db = db_path();
    let rows = tauri::async_runtime::spawn_blocking(move || crate::db::progress::list_bookmarks(&db, &track_id))
        .await
        .map_err(|e| IpcError::join_failed("读书签", e))?
        .map_err(IpcError::db)?;
    Ok(rows)
}

#[tauri::command]
pub async fn add_bookmark(track_id: String, at_ms: i64, label: String) -> IpcResult<Option<Bookmark>> {
    if track_id.is_empty() || at_ms < 0 {
        return Err(IpcError::invalid_argument("曲目 ID 不能为空，时间点不能为负"));
    }
    let db = db_path();
    let created = tauri::async_runtime::spawn_blocking(move || crate::db::progress::add_bookmark(&db, &track_id, at_ms, &label))
        .await
        .map_err(|e| IpcError::join_failed("写书签", e))?
        .map_err(IpcError::db)?;
    Ok(created)
}

#[tauri::command]
pub async fn rename_bookmark(id: String, label: String) -> IpcResult<bool> {
    if id.is_empty() {
        return Err(IpcError::invalid_argument("书签 ID 不能为空"));
    }
    let db = db_path();
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::rename_bookmark(&db, &id, &label))
        .await
        .map_err(|e| IpcError::join_failed("改书签名", e))?
        .map_err(IpcError::db)?;
    Ok(true)
}

#[tauri::command]
pub async fn remove_bookmark(id: String) -> IpcResult<bool> {
    if id.is_empty() {
        return Err(IpcError::invalid_argument("书签 ID 不能为空"));
    }
    let db = db_path();
    tauri::async_runtime::spawn_blocking(move || crate::db::progress::remove_bookmark(&db, &id))
        .await
        .map_err(|e| IpcError::join_failed("删书签", e))?
        .map_err(IpcError::db)?;
    Ok(true)
}

#[tauri::command]
pub fn start_watching(paths: Vec<String>, scope: State<'_, PathScope>, state: State<'_, AppWindowsState>) -> IpcResult<bool> {
    // 退出途中不再挂监听（P1-3 第一步「停止接收」）：挂上也会被收尾的下一步停掉
    if !crate::shutdown::accepting() {
        return Err(crate::error::IpcError::new(crate::error::AppErrorCode::Busy, "应用正在退出，未开始监听"));
    }
    // 逐条筛（P0-1）：监听器只该挂在授权根上；离线盘的根解析不出来，本来也盯不住
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|p| {
            let ok = scope.allows(p);
            if !ok {
                crate::paths::append_log(&format!("[scope] start_watching 跳过 {p}"));
            }
            ok
        })
        .collect();
    // 挂不上的根（离线盘/目录不存在）逐条记日志，命令层如实回 false —— 不再无条件报成功
    let failed = crate::watcher::start(&paths, &state.watcher)?;
    if !failed.is_empty() {
        crate::paths::append_log(&format!(
            "[watcher] start_watching 有 {}/{} 个根没挂上：{}",
            failed.len(),
            paths.len(),
            failed.join("、")
        ));
    }
    Ok(failed.is_empty())
}

#[tauri::command]
pub fn stop_watching(state: State<'_, AppWindowsState>) -> bool {
    crate::watcher::stop(&state.watcher);
    true
}
