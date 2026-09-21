//! 命令层：播放控制（前端遥控器 → mpv）。

use crate::error::IpcResult;
use crate::model::{AudioDevice, PlayerState, SubtitleTrack, SubStyle};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::join_task;
use crate::player::PlayerCore;
use crate::state::AppWindowsState;

/// 参数表就是 IPC 契约本身（`contracts/commands.ts` 的 `player_play.args` 一一对应），
/// 收进结构体要同时改前端调用点与 `docs/IPC.md`，随 S3 一起做，这里先不凑数。
#[allow(clippy::too_many_arguments)]
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
) -> IpcResult<()> {
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
pub async fn player_toggle(app: AppHandle, paused: bool) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("切换播放/暂停", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_paused(paused)).await)
}

#[tauri::command]
pub async fn player_seek(app: AppHandle, sec: f64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task(
        "跳转进度",
        tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().seek(sec.clamp(0.0, 86400.0 * 24.0))).await,
    )
}

#[tauri::command]
pub async fn player_set_volume(app: AppHandle, vol: f64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("设置音量", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_volume(vol.clamp(0.0, 1.0))).await)
}

#[tauri::command]
pub async fn player_set_muted(app: AppHandle, muted: bool) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("设置静音", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_muted(muted)).await)
}

#[tauri::command]
pub async fn player_set_speed(app: AppHandle, speed: f64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("设置倍速", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_speed(speed.clamp(0.5, 2.0))).await)
}

#[tauri::command]
pub async fn player_stop(app: AppHandle) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("停止播放", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().stop()).await)
}

#[tauri::command]
pub async fn player_set_video_enabled(app: AppHandle, enabled: bool) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("切换画面开关", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_video_enabled(enabled)).await)
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
pub async fn player_set_subtitle_track(app: AppHandle, id: i64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("选择字幕轨", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_subtitle_track(id)).await)
}

#[tauri::command]
pub async fn player_set_subtitle_visible(app: AppHandle, visible: bool) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("显示/隐藏字幕", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_subtitle_visible(visible)).await)
}

#[tauri::command]
pub async fn player_set_subtitle_delay(app: AppHandle, sec: f64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task(
        "设置字幕延迟",
        tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_subtitle_delay(sec.clamp(-600.0, 600.0))).await,
    )
}

#[tauri::command]
pub async fn player_set_sub_style(app: AppHandle, style: SubStyle) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("设置字幕样式", tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().set_sub_style(style)).await)
}

#[tauri::command]
pub async fn player_apply_loudness_gain(
    app: AppHandle,
    track_lufs: Option<f64>,
    target_lufs: f64,
) -> IpcResult<()> {
    let app2 = app.clone();
    join_task(
        "应用响度增益",
        tauri::async_runtime::spawn_blocking(move || app2.state::<PlayerCore>().apply_loudness_gain(track_lufs, target_lufs)).await,
    )
}

/// 舞台几何：只投递命令（不阻塞），Win32 操作由 kx-stage 线程执行。
/// 命令来源窗口决定它有没有资格改动画面层：悬浮窗接管期间主窗口一律被忽略。
#[tauri::command]
pub async fn player_set_stage_rect(
    window: WebviewWindow,
    app: AppHandle,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    visible: bool,
) -> bool {
    let from_pip = window.label() == "pip";
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app2.state::<PlayerCore>().set_stage_rect(x, y, w, h, visible, from_pip);
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
// 悬浮窗是独立的系统窗口（label="pip"），mpv 宿主窗口「垫」在它之下（underlay）：
// 视频从浮窗的透明像素透出，浮窗内的 DOM 控件因此能浮在画面上。
// 几何由 pip 窗口内的 PipRoot 以「相对 pip 客户区」的物理像素下发，
// 与主窗口舞台互斥（挂靠期间主窗口不再有权改动画面层，见 set_stage_rect）。
//
// 初始化握手流程（避免 pip:shown 在 WebView 加载完成前被丢弃）：
//   1. pip_open  → 创建/定位/显示窗口（不做 attach，不 emit pip:shown）
//   2. PipRoot.onMounted → api.pipReady()
//   3. pip_ready → attach_overlay(hwnd) → emit("pip:shown")
//   4. PipRoot 收到 pip:shown → syncRect()

/// 打开/显示独立悬浮窗。x/y/w/h 为物理像素；窗口复用（首次创建后只显示+定位）。
/// 不在此处 attach_overlay，等 pip_ready 握手后再做，确保 WebView 已就绪。
#[tauri::command]
pub async fn pip_open(app: AppHandle, x: i32, y: i32, w: u32, h: u32) -> bool {
    let is_new = app.get_webview_window("pip").is_none();
    let pip = match app.get_webview_window("pip") {
        Some(win) => win,
        None => match WebviewWindowBuilder::new(
            &app,
            "pip",
            WebviewUrl::App("index.html".into()),
        )
        .title("KX 悬浮窗")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        // 窗口 = 画面 + 四周留白，且比例锁死，所以下限只要不妨碍最小的预设档即可
        .min_inner_size(120.0, 80.0)
        .focused(false)
        .build()
        {
            Ok(win) => win,
            Err(e) => {
                crate::paths::append_log(&format!("[pip] 创建悬浮窗失败: {e}"));
                app.state::<PlayerCore>().mark_pip_active(false);
                return false;
            }
        },
    };
    // 必须在 show() 之前声明：显示浮窗会让主窗口失焦，那次 Focused(false) 早于
    // attach 到达的话会把视频层一起隐藏（小窗只剩声音）。
    app.state::<PlayerCore>().mark_pip_active(true);
    let _ = pip.show();
    let _ = pip.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = pip.set_size(tauri::PhysicalSize::new(w, h));
    // 窗口复用（WebView 已运行）：直接 attach + 通知 PipRoot 重新对齐
    // 窗口首次创建：WebView 还在加载，等 pip_ready 握手
    if !is_new {
        let hwnd = pip.hwnd().ok().map(|h| h.0 as isize);
        let player = app.state::<PlayerCore>();
        player.attach_overlay(hwnd);
        let _ = app.emit("pip:shown", ());
    }
    // 首次创建：pip_ready 会由 PipRoot.onMounted 触发，届时再 attach + emit
    true
}

/// PipRoot.onMounted 握手：WebView 已就绪，现在安全地做 attach_overlay 并通知对齐。
#[tauri::command]
pub async fn pip_ready(app: AppHandle) -> bool {
    let pip = match app.get_webview_window("pip") {
        Some(w) => w,
        None => return false,
    };
    let hwnd = pip.hwnd().ok().map(|h| h.0 as isize);
    let player = app.state::<PlayerCore>();
    player.attach_overlay(hwnd);
    let _ = app.emit("pip:shown", ());
    true
}

/// 关闭/隐藏悬浮窗，mpv 覆盖窗口挂回主窗口。
#[tauri::command]
pub async fn pip_close(app: AppHandle) -> bool {
    {
        let player = app.state::<PlayerCore>();
        // 先把覆盖窗口挂回主窗口（last_rect 由前端在舞台视图时重新下发，不在此清除）
        player.detach_overlay();
    }
    if let Some(pip) = app.get_webview_window("pip") {
        let _ = pip.hide();
    }
    let _ = app.emit("pip:closed", ());
    true
}

/// 从悬浮窗还原回主窗口舞台：覆盖窗口挂回主窗口、隐藏浮窗、主窗口唤到前台，
/// 并广播 `pip:restored`（前端监听后把视图切回舞台）。
#[tauri::command]
pub async fn pip_restore(app: AppHandle) -> bool {
    {
        let player = app.state::<PlayerCore>();
        player.detach_overlay();
    }
    if let Some(pip) = app.get_webview_window("pip") {
        let _ = pip.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
    let _ = app.emit("pip:closed", ());
    let _ = app.emit("pip:restored", ());
    true
}

/// 钉住状态变更（pip 窗口内切换时同步到主窗口）
#[tauri::command]
pub async fn pip_set_pinned(app: AppHandle, pinned: bool) -> bool {
    let _ = app.emit("pip:pinned", pinned);
    true
}
