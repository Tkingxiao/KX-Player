//! 命令层：播放控制（前端遥控器 → mpv）。

use crate::model::{AudioDevice, PlayerState, SubtitleTrack, SubStyle};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::player::PlayerCore;
use crate::state::AppWindowsState;

#[tauri::command]
pub async fn player_play(
    window: WebviewWindow,
    state: State<'_, PlayerCore>,
    track_path: String,
    resume_ms: Option<f64>,
    vol: Option<f64>,
    muted: Option<bool>,
    speed: Option<f64>,
    video_active: Option<bool>,
) -> Result<(), String> {
    state.ensure_started(&window)?;
    state.play_file(
        &track_path,
        resume_ms.unwrap_or(0.0),
        vol.unwrap_or(0.85).clamp(0.0, 1.0),
        muted.unwrap_or(false),
        speed.unwrap_or(1.0).clamp(0.5, 2.0),
        video_active.unwrap_or(false),
    )
}

// 以下播放命令全部为 async：它们会调用 libmpv（可能阻塞）或投递窗口命令。
// 同步命令在 Tauri 主线程的窗口消息循环上执行，一旦阻塞整个 UI 就「未响应」；
// async 命令由 Tauri 派发到异步运行时（配合 spawn_blocking），主线程不参与，界面永远可响应。
// 注意：async 命令取 AppHandle（自有），在闭包内再取 state（满足 'static）。

#[tauri::command]
pub async fn player_toggle(app: AppHandle, paused: bool) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_paused(paused))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_seek(app: AppHandle, sec: f64) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().seek(sec.clamp(0.0, 86400.0 * 24.0)))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_volume(app: AppHandle, vol: f64) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_volume(vol.clamp(0.0, 1.0)))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_muted(app: AppHandle, muted: bool) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_muted(muted))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_speed(app: AppHandle, speed: f64) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_speed(speed.clamp(0.5, 2.0)))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_stop(app: AppHandle) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().stop())
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_video_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_video_enabled(enabled))
        .await
        .unwrap_or(Err("任务失败".into()))
}

// ── 字幕子系统（mpv 自带 libass，这里做属性遥控）──

#[tauri::command]
pub async fn player_subtitle_tracks(app: AppHandle) -> Vec<SubtitleTrack> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().subtitle_tracks())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn player_set_subtitle_track(app: AppHandle, id: i64) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_subtitle_track(id))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_subtitle_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_subtitle_visible(visible))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_subtitle_delay(app: AppHandle, sec: f64) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app2.state::<PlayerCore>().set_subtitle_delay(sec.clamp(-600.0, 600.0))
    })
    .await
    .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_set_sub_style(app: AppHandle, style: SubStyle) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_sub_style(style))
        .await
        .unwrap_or(Err("任务失败".into()))
}

#[tauri::command]
pub async fn player_apply_loudness_gain(
    app: AppHandle,
    track_lufs: Option<f64>,
    target_lufs: f64,
) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app2.state::<PlayerCore>().apply_loudness_gain(track_lufs, target_lufs)
    })
    .await
    .unwrap_or(Err("任务失败".into()))
}

/// 舞台几何：只投递命令（不阻塞），Win32 操作由 kx-stage 线程执行
#[tauri::command]
pub async fn player_set_stage_rect(
    app: AppHandle,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    visible: bool,
) -> bool {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app2.state::<PlayerCore>().set_stage_rect(x, y, w, h, visible);
    })
    .await
    .is_ok()
}

#[tauri::command]
pub async fn player_get_state(app: AppHandle) -> PlayerState {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().snapshot())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn player_list_devices(app: AppHandle) -> Vec<AudioDevice> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().list_devices())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn player_set_device(app: AppHandle, device_id: String) -> bool {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_device(&device_id).is_ok())
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub fn app_force_quit(app: tauri::AppHandle, state: State<'_, AppWindowsState>) {
    state.force_close.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = app.emit("window:beforeClose", ());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        crate::fftools::kill_all();
        app.exit(0);
    });
}

// ── 独立悬浮窗（画中画）──────────────────────────────────────
// 悬浮窗是独立的系统窗口（label="pip"），mpv 覆盖窗口动态挂靠到它上面：
// 几何由 pip 窗口内的 PipRoot 以「相对 pip 客户区」的物理像素下发，
// 与主窗口舞台互斥（挂靠时主窗口不再下发舞台矩形）。

/// 打开/显示独立悬浮窗。x/y/w/h 为物理像素；窗口复用（首次创建后只显示+定位）。
#[tauri::command]
pub async fn pip_open(app: AppHandle, x: i32, y: i32, w: u32, h: u32) -> bool {
    let pip = match app.get_webview_window("pip") {
        Some(win) => win,
        None => match WebviewWindowBuilder::new(
            &app,
            "pip",
            WebviewUrl::App("index.html#/pip".into()),
        )
        .title("KX 悬浮窗")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .min_inner_size(200.0, 150.0)
        .focused(false)
        .build()
        {
            Ok(win) => win,
            Err(e) => {
                crate::paths::append_log(&format!("[pip] 创建悬浮窗失败: {e}"));
                return false;
            }
        },
    };
    let _ = pip.show();
    let _ = pip.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = pip.set_size(tauri::PhysicalSize::new(w, h));
    let hwnd = pip.hwnd().ok().map(|h| h.0 as isize);
    let player = app.state::<PlayerCore>();
    player.attach_overlay(hwnd);
    // 通知 pip 窗口内的 PipRoot：窗口已显示，重新对齐 mpv 覆盖窗口
    let _ = app.emit("pip:shown", ());
    true
}

/// 关闭/隐藏悬浮窗，mpv 覆盖窗口挂回主窗口。
#[tauri::command]
pub async fn pip_close(app: AppHandle) -> bool {
    {
        let player = app.state::<PlayerCore>();
        player.attach_overlay(None);
    }
    if let Some(pip) = app.get_webview_window("pip") {
        let _ = pip.hide();
    }
    let _ = app.emit("pip:closed", ());
    true
}

/// 钉住状态变更（pip 窗口内切换时同步到主窗口）
#[tauri::command]
pub async fn pip_set_pinned(app: AppHandle, pinned: bool) -> bool {
    let _ = app.emit("pip:pinned", pinned);
    true
}
