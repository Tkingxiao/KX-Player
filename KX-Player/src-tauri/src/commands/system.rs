//! 命令层：设置/背景图/窗口/ffmpeg/AI。

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::{AiChatPayload, AiChatResult, AiModelsResult, AiPingResult, BgImageData};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use super::join_task;
use crate::state::AppWindowsState;

// ── 设置 ────────────────────────────────────────────────────────

#[tauri::command]
pub fn load_settings() -> Value {
    crate::settingsio::load(&crate::paths::settings_path())
}

#[tauri::command]
pub fn save_settings(settings: Value) -> bool {
    crate::settingsio::save(&crate::paths::settings_path(), &settings)
}

// ── 背景图 ──────────────────────────────────────────────────────

#[tauri::command]
pub fn load_bg_image() -> Option<BgImageData> {
    crate::bgimage::load()
}

#[tauri::command]
pub fn save_bg_image(data_url: String) -> bool {
    crate::bgimage::save_data_url(&data_url)
}

#[tauri::command]
pub fn remove_bg_image() -> bool {
    crate::bgimage::remove()
}

// ── 启动自检 ────────────────────────────────────────────────────

/// 取走 setup 阶段收集的降级提示；取一次即清空，重启才会再报。
#[tauri::command]
pub fn startup_warnings() -> Vec<String> {
    crate::state::take_startup_warnings()
}

// ── 窗口 ────────────────────────────────────────────────────────

#[tauri::command]
pub fn minimize_window(window: WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
pub fn maximize_window(window: WebviewWindow) {
    match window.is_maximized() {
        Ok(true) => {
            let _ = window.unmaximize();
        }
        _ => {
            let _ = window.maximize();
        }
    }
}

#[tauri::command]
pub fn close_window(window: WebviewWindow) {
    // 关闭 = 隐藏到托盘
    let _ = window.hide();
}

#[tauri::command]
pub fn is_maximized(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

#[tauri::command]
pub fn force_close_window(app: AppHandle, window: WebviewWindow, state: State<'_, AppWindowsState>) {
    state.force_close.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = app.emit("window:beforeClose", ());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = window.close();
    });
}

#[tauri::command]
pub fn toggle_fullscreen(window: WebviewWindow) -> IpcResult<bool> {
    let q = |e: tauri::Error| IpcError::with_detail(AppErrorCode::Internal, "切换全屏失败", e);
    let on = !window.is_fullscreen().map_err(q)?;
    window.set_fullscreen(on).map_err(q)?;
    Ok(on)
}

// ── ffmpeg ──────────────────────────────────────────────────────

#[tauri::command]
pub fn ffmpeg_exec(args: Vec<String>) -> crate::fftools::ExecResult {
    crate::fftools::exec_ffmpeg(&args)
}

/// ffmpeg 可用性探测（转换页据此降级）
#[tauri::command]
pub fn ffmpeg_probe() -> crate::fftools::FfmpegInfo {
    crate::fftools::probe_ffmpeg()
}

/// 启动转换队列（任务 + convert:progress 事件），返回 taskId
#[tauri::command]
pub fn convert_run(app: AppHandle, items: Vec<crate::fftools::ConvertItem>) -> u64 {
    crate::fftools::run_convert_queue(app, items)
}

/// 取消转换队列（幂等）
#[tauri::command]
pub fn convert_cancel(task_id: u64) -> bool {
    crate::fftools::cancel_convert_queue(task_id)
}

// ── AI ──────────────────────────────────────────────────────────

/// 单次补全。**async 不是风格选择**：同步命令跑在主线程，120 s 的超时会把整个 UI 冻住，
/// 而且用户点不了取消（B3）。批量翻译走 `ai_translate_start` 的任务 + 事件通道。
#[tauri::command]
pub async fn ai_chat(payload: AiChatPayload) -> AiChatResult {
    if payload.messages.is_empty() {
        return AiChatResult { ok: false, content: String::new(), error: Some("参数错误".into()) };
    }
    match crate::ai::chat_completion(&payload, 120_000).await {
        Ok(content) => AiChatResult { ok: true, content, error: None },
        Err(e) => AiChatResult { ok: false, content: String::new(), error: Some(e.text()) },
    }
}

#[tauri::command]
pub async fn ai_ping(payload: AiChatPayload) -> AiPingResult {
    crate::ai::ping(&payload).await
}

#[tauri::command]
pub async fn ai_list_models(base_url: String, api_key: String) -> AiModelsResult {
    crate::ai::list_models(&base_url, &api_key).await
}

/// 启动字幕翻译任务（返回 taskId；进度与结果走 ai:progress 事件，见 B3）
#[tauri::command]
pub fn ai_translate_start(app: AppHandle, spec: crate::ai_job::AiTranslateSpec) -> u64 {
    crate::ai_job::run_translate(app, spec)
}

/// 取消翻译任务（幂等；块级粒度：在飞的那一块跑完就停）
#[tauri::command]
pub fn ai_task_cancel(task_id: u64) -> bool {
    crate::ai_job::cancel(task_id)
}

/// 启动纯文本批量翻译任务（AI 页的「文件夹名翻译」）。
/// 与 `ai_translate_start` 同一条 `ai:progress` 事件流，收尾事件带 `translated`。
#[tauri::command]
pub fn ai_texts_start(app: AppHandle, spec: crate::ai_job::AiTextsSpec) -> u64 {
    crate::ai_job::run_texts(app, spec)
}

/// AI-16 导出：把选中的字幕切成 `### 序号 ###` 批次写成 prompts.txt，位置由用户在保存框里定。
/// 整条链路不碰 Provider（`05` 第 16 条要的兜底路径），参数只有路径列表。
/// 保存框被取消回 `CANCELLED`（前端据此不出 Toast），写盘失败才回 `IO_FAILED`。
#[tauri::command]
pub async fn ai_export_prompts(app: AppHandle, paths: Vec<String>) -> IpcResult<crate::ai_export::ExportResult> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中的字幕文件"));
    }
    join_task(
        "导出翻译批次",
        tauri::async_runtime::spawn_blocking(move || {
            let loaded = crate::ai_export::load(&paths);
            if loaded.files.is_empty() {
                return Err(IpcError::invalid_argument("这些文件里没有可导出的字幕文本"));
            }
            let content = crate::ai_export::build_prompt(&loaded);
            if content.is_empty() {
                // 全批没有一条能安全切块 —— 开保存框只会得到一个空文件
                return Err(IpcError::invalid_argument("每条原文都自带 `### 数字 ###` 形状的行，无法切块导出"));
            }
            let Some(picked) = app
                .dialog()
                .file()
                .set_title("导出待翻译批次")
                .set_file_name("prompts.txt")
                .add_filter("文本", &["txt"])
                .blocking_save_file()
            else {
                return Err(IpcError::new(AppErrorCode::Cancelled, "已取消导出"));
            };
            let path = picked.to_string();
            if let Err(e) = std::fs::write(&path, &content) {
                return Err(IpcError::with_detail(AppErrorCode::IoFailed, "写入批次文件失败", e));
            }
            Ok(crate::ai_export::export_meta(&loaded, &path))
        })
        .await,
    )
}

/// AI-16 导入：把手工跑出的译文并回选中的字幕文件。`apply = false` 只做校验和差异预览、
/// 一点不碰磁盘 —— 预览与落地是同一个函数，所以「报可以写几篇」和「实际写了几篇」不会脱节。
/// 译文文本由前端读（`read_text_file` 管编码），这里不再开第二次对话框。
#[tauri::command]
pub async fn ai_import_translations(paths: Vec<String>, text: String, apply: bool) -> IpcResult<crate::ai_export::ImportResult> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中的字幕文件"));
    }
    if text.trim().is_empty() {
        return Err(IpcError::invalid_argument("译文是空的"));
    }
    join_task(
        "导入翻译批次",
        tauri::async_runtime::spawn_blocking(move || {
            let loaded = crate::ai_export::load(&paths);
            if loaded.files.is_empty() {
                return Err(IpcError::invalid_argument("这些文件里没有可写入的字幕文本"));
            }
            Ok(crate::ai_export::import(&loaded, &text, apply))
        })
        .await,
    )
}

/// 递归扫描目录（subtitle = 字幕文件，dir = 子目录）。替代前端的 api.listDir 递归：
/// 旧实现每层一次 IPC，一个音乐库目录树要几百次往返，且顺序随 readdir 漂移。
/// async + spawn_blocking：整棵树的 readdir 是 IO，同步命令会在主线程跑（见 `commands/library.rs` 的口径）。
#[tauri::command]
pub async fn subtitle_scan_dir(
    dir: String,
    kind: String,
    max_depth: Option<usize>,
    limit: Option<usize>,
) -> Vec<crate::ai_scan::ScannedItem> {
    let kind = match kind.as_str() {
        "dir" => crate::ai_scan::ScanKind::Dir,
        _ => crate::ai_scan::ScanKind::Subtitle,
    };
    // 深度与数量的缺省值来自旧前端的两处递归：字幕 4/500、目录 3/300
    let (default_depth, default_limit) = match kind {
        crate::ai_scan::ScanKind::Dir => (3, 300),
        crate::ai_scan::ScanKind::Subtitle => (4, 500),
    };
    let max_depth = max_depth.unwrap_or(default_depth);
    let limit = limit.unwrap_or(default_limit);
    match tauri::async_runtime::spawn_blocking(move || {
        crate::ai_scan::scan_dir(&dir, kind, max_depth, limit)
    })
    .await
    {
        Ok(items) => items,
        Err(e) => {
            crate::paths::append_log(&format!("[ai] subtitle_scan_dir 阻塞任务失败: {e}"));
            vec![]
        }
    }
}
