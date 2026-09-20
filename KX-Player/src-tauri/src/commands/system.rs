//! 命令层：设置/背景图/窗口/ffmpeg/AI。

use crate::model::{AiChatPayload, AiChatResult, AiModelsResult, AiPingResult, BgImageData};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State, WebviewWindow};

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
pub fn toggle_fullscreen(window: WebviewWindow) -> Result<bool, String> {
    let on = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(on).map_err(|e| e.to_string())?;
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

#[tauri::command]
pub fn ai_chat(payload: AiChatPayload) -> AiChatResult {
    if payload.messages.is_empty() {
        return AiChatResult { ok: false, content: String::new(), error: Some("参数错误".into()) };
    }
    match crate::ai::chat_completion(&payload, 120_000) {
        Ok(content) => AiChatResult { ok: true, content, error: None },
        Err(e) => AiChatResult { ok: false, content: String::new(), error: Some(e) },
    }
}

#[tauri::command]
pub fn ai_ping(payload: AiChatPayload) -> AiPingResult {
    crate::ai::ping(&payload)
}

#[tauri::command]
pub fn ai_list_models(base_url: String, api_key: String) -> AiModelsResult {
    crate::ai::list_models(&base_url, &api_key)
}
