//! 播放内核：libmpv2 适配层。mpv 是播放状态的唯一事实源；
//! Rust 只转发命令、订阅属性并归一化成 `player:state` / `player:ended` 等事件。

pub mod embed;

use crate::model::PlayerState;
use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, WebviewWindow};

pub struct PlayerCore {
    pub app: AppHandle,
    mpv: OnceCell<Arc<Mpv>>,
    host: OnceCell<isize>,
    main_hwnd: OnceCell<isize>,
    start_lock: Mutex<()>,
    pub shared: Arc<Mutex<PlayerState>>,
    last_emit_ms: AtomicU64,
    /// 最后已知的舞台矩形（主窗口移动/缩放/聚焦时用于重放）
    last_rect: parking_lot::Mutex<Option<embed::StageRect>>,
}

impl PlayerCore {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            mpv: OnceCell::new(),
            host: OnceCell::new(),
            main_hwnd: OnceCell::new(),
            start_lock: Mutex::new(()),
            shared: Arc::new(Mutex::new(PlayerState {
                volume: 85.0,
                ..Default::default()
            })),
            last_emit_ms: AtomicU64::new(0),
            last_rect: parking_lot::Mutex::new(None),
        }
    }

    fn emit_state(&self, force: bool) {
        let now = now_ms();
        let last = self.last_emit_ms.load(Ordering::Relaxed);
        if !force && now - last < 100 {
            return;
        }
        self.last_emit_ms.store(now, Ordering::Relaxed);
        let snap = self.shared.lock().clone();
        let _ = self.app.emit("player:state", snap);
    }

    /// 首次播放时初始化 mpv 与嵌入窗口（幂等）
    pub fn ensure_started(&self, window: &WebviewWindow) -> Result<(), String> {
        if self.mpv.get().is_some() {
            return Ok(());
        }
        let _g = self.start_lock.lock();
        if self.mpv.get().is_some() {
            return Ok(());
        }
        let main_hwnd: isize = window
            .hwnd()
            .map_err(|e| format!("获取窗口句柄失败: {e}"))?
            .0 as isize;
        // 舞台线程：创建覆盖窗口并跑消息循环（窗口操作只在该线程执行，避免跨线程 SendMessage 死锁）
        let host = embed::start_stage_thread(main_hwnd).ok_or("创建 mpv 宿主窗口失败")?;
        let mpv = Mpv::with_initializer(|init| {
            init.set_option("wid", host as i64)?;
            init.set_option("vo", "gpu-next".to_string())?;
            init.set_option("hwdec", "auto-safe".to_string())?;
            init.set_option("gpu-api", "d3d11".to_string())?;
            init.set_option("keep-open", "yes".to_string())?;
            init.set_option("force-window", "no".to_string())?;
            init.set_option("osc", "no".to_string())?;
            init.set_option("input-default-bindings", "no".to_string())?;
            init.set_option("input-vo-keyboard", "no".to_string())?;
            init.set_option("audio-display", "no".to_string())?;
            Ok(())
        })
        .map_err(|e| format!("mpv 初始化失败: {e}"))?;
        let mpv = Arc::new(mpv);

        let _ = self.main_hwnd.set(main_hwnd);
        let _ = self.host.set(host);
        let _ = self.mpv.set(Arc::clone(&mpv));
        // 竞态补偿：前端可能在 mpv 初始化完成前就发来舞台矩形（那时 host 还是 None），
        // 此时补应用一次，否则画面窗口会一直停在初始化时的离屏位置（表现为黑屏）。
        self.reposition();

        // 事件线程：属性观察 → 归一化 player:state
        let app = self.app.clone();
        let shared = Arc::clone(&self.shared);
        let last_emit = Arc::new(AtomicU64::new(0));
        std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn(move || event_loop(mpv, app, shared, last_emit))
            .map_err(|e| format!("mpv 事件线程启动失败: {e}"))?;
        crate::paths::append_log("[mpv] initialized with embedded window");
        Ok(())
    }

    pub fn play_file(&self, path: &str, resume_ms: f64, vol: f64, muted: bool, speed: f64, video_active: bool) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        {
            let mut s = self.shared.lock();
            s.playing = true;
            s.position = 0.0;
            s.duration = 0.0;
            s.track_path = Some(path.to_string());
            s.video_active = video_active;
        }
        mpv.set_property("volume", vol * 100.0).map_err(mpv_err)?;
        mpv.set_property("mute", muted).map_err(mpv_err)?;
        mpv.set_property("speed", speed).map_err(mpv_err)?;
        if resume_ms > 0.0 {
            mpv.set_property("start", resume_ms / 1000.0).map_err(mpv_err)?;
        } else {
            let _ = mpv.set_property("start", 0.0_f64);
        }
        mpv.command("loadfile", &[path]).map_err(mpv_err)?;
        self.emit_state(true);
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("pause", paused).map_err(mpv_err)
    }

    pub fn seek(&self, sec: f64) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.command("seek", &[&format!("{sec:.3}"), "absolute"]).map_err(mpv_err)
    }

    pub fn set_volume(&self, v: f64) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("volume", v * 100.0).map_err(mpv_err)
    }

    pub fn set_muted(&self, m: bool) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("mute", m).map_err(mpv_err)
    }

    pub fn set_speed(&self, s: f64) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("speed", s).map_err(mpv_err)
    }

    pub fn stop(&self) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.command("stop", &[]).map_err(mpv_err)?;
        {
            let mut s = self.shared.lock();
            s.playing = false;
            s.position = 0.0;
            s.track_path = None;
            s.video_active = false;
        }
        self.set_stage_visible(false);
        self.emit_state(true);
        Ok(())
    }

    /// 「仅音频」= vid=no（引擎始终是 mpv，播放位置不丢）
    pub fn set_video_enabled(&self, enabled: bool) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("vid", if enabled { "auto".to_string() } else { "no".to_string() })
            .map_err(mpv_err)?;
        {
            self.shared.lock().video_active = enabled;
        }
        self.emit_state(true);
        Ok(())
    }

    pub fn set_stage_rect(&self, x: i32, y: i32, w: i32, h: i32, visible: bool) {
        let rect = embed::StageRect { x, y, w, h, visible };
        *self.last_rect.lock() = Some(rect);
        embed::set_stage_rect(rect);
    }

    pub fn set_stage_visible(&self, visible: bool) {
        let rect = {
            let mut guard = self.last_rect.lock();
            let r = guard.get_or_insert(embed::StageRect { x: 0, y: 0, w: 0, h: 0, visible });
            r.visible = visible;
            *r
        };
        embed::set_stage_rect(rect);
    }

    /// 主窗口移动/缩放后重放最后已知的舞台矩形（覆盖窗口按屏幕坐标定位，客户区矩形不变）
    pub fn reposition(&self) {
        if let Some(rect) = *self.last_rect.lock() {
            embed::set_stage_rect(rect);
        }
    }

    /// 主窗口失焦时只隐藏覆盖窗口（矩形状态保留）；聚焦时恢复。
    pub fn focus_changed(&self, focused: bool) {
        if focused {
            if let Some(rect) = *self.last_rect.lock() {
                embed::set_stage_rect(rect);
            }
        } else {
            embed::set_os_visible_cmd(false);
        }
    }

    /// 销毁覆盖窗口（应用退出时）
    pub fn destroy_host(&self) {
        embed::destroy();
    }

    pub fn list_devices(&self) -> Vec<crate::model::AudioDevice> {
        let mpv = match self.mpv.get() {
            Some(m) => m,
            None => return vec![default_device()],
        };
        let raw: String = match mpv.get_property("audio-device-list") {
            Ok(s) => s,
            Err(_) => return vec![default_device()],
        };
        // mpv 的字符串化格式：name1/desc1,name2/desc2,...
        let mut devices = vec![default_device()];
        for pair in raw.split(',') {
            if let Some((name, desc)) = pair.split_once('/') {
                if name == "auto" || name.is_empty() {
                    continue;
                }
                devices.push(crate::model::AudioDevice { device_id: name.to_string(), label: desc.to_string() });
            }
        }
        devices
    }

    pub fn set_device(&self, id: &str) -> Result<(), String> {
        let mpv = self.mpv.get().ok_or("mpv 未初始化")?;
        mpv.set_property("audio-device", id.to_string()).map_err(mpv_err)
    }

    pub fn snapshot(&self) -> PlayerState {
        self.shared.lock().clone()
    }
}

fn default_device() -> crate::model::AudioDevice {
    crate::model::AudioDevice { device_id: "auto".into(), label: "系统默认输出".into() }
}

fn mpv_err(e: libmpv2::Error) -> String {
    format!("mpv: {e}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn event_loop(mpv: Arc<Mpv>, app: AppHandle, shared: Arc<Mutex<PlayerState>>, _last_emit: Arc<AtomicU64>) {
    let _ = mpv.disable_deprecated_events();
    let _ = mpv.observe_property("time-pos", Format::Double, 1);
    let _ = mpv.observe_property("duration", Format::Double, 2);
    let _ = mpv.observe_property("pause", Format::Flag, 3);
    let _ = mpv.observe_property("speed", Format::Double, 4);
    let _ = mpv.observe_property("volume", Format::Double, 5);
    let _ = mpv.observe_property("mute", Format::Flag, 6);
    let _ = mpv.enable_event(libmpv2::events::mpv_event_id::StartFile);
    let _ = mpv.enable_event(libmpv2::events::mpv_event_id::EndFile);
    let _ = mpv.enable_event(libmpv2::events::mpv_event_id::FileLoaded);
    let _ = mpv.enable_event(libmpv2::events::mpv_event_id::Shutdown);

    loop {
        match mpv.wait_event(0.25) {
            Some(Ok(Event::PropertyChange { name, change, .. })) => {
                let mut force = false;
                {
                    let mut s = shared.lock();
                    match change {
                        PropertyData::Double(v) if name == "time-pos" => {
                            s.position = v.max(0.0);
                        }
                        PropertyData::Double(v) if name == "duration" => {
                            if (s.duration - v).abs() > 0.01 {
                                s.duration = v.max(0.0);
                                force = true;
                            }
                        }
                        PropertyData::Flag(p) if name == "pause" => {
                            if s.playing == p {
                                s.playing = !p;
                                force = true;
                            }
                        }
                        PropertyData::Double(v) if name == "speed" => {
                            if (s.speed - v).abs() > 0.001 {
                                s.speed = v;
                                force = true;
                            }
                        }
                        PropertyData::Double(v) if name == "volume" => {
                            s.volume = v.clamp(0.0, 100.0);
                        }
                        PropertyData::Flag(m) if name == "mute" => {
                            if s.muted != m {
                                s.muted = m;
                                force = true;
                            }
                        }
                        _ => {}
                    }
                }
                emit_state_shared(&app, &shared, force, &mut None, Duration::from_millis(100));
            }
            Some(Ok(Event::StartFile)) => {
                {
                    let mut s = shared.lock();
                    s.playing = true;
                    s.position = 0.0;
                }
                let _ = app.emit("player:loading", ());
                emit_state_shared(&app, &shared, true, &mut None, Duration::from_millis(0));
            }
            Some(Ok(Event::FileLoaded)) => {
                // 用 start 属性实现的续播已生效；复位以免影响下一次加载
                let _ = mpv.set_property("start", 0.0_f64);
            }
            Some(Ok(Event::EndFile(reason))) => {
                let code = reason as i32;
                let payload = match code {
                    0 => "eof",
                    2 => "stop",
                    3 => "quit",
                    4 => "error",
                    _ => "unknown",
                };
                let _ = app.emit("player:ended", serde_json::json!({ "reason": payload }));
            }
            Some(Ok(Event::Shutdown)) => {
                let _ = app.emit("player:error", serde_json::json!({ "message": "mpv 内核已关闭" }));
                break;
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                crate::paths::append_log(&format!("[mpv] event error: {e}"));
            }
            None => {
                // 超时：刷新一次状态（保持进度条前进）
                emit_state_shared(&app, &shared, false, &mut None, Duration::from_millis(250));
            }
        }
    }
}

/// 共享状态的节流广播（事件线程用；与 PlayerCore::emit_state 同语义但免借用冲突）
fn emit_state_shared(
    app: &AppHandle,
    shared: &Arc<Mutex<PlayerState>>,
    force: bool,
    last: &mut Option<Instant>,
    min_interval: Duration,
) {
    let _ = last;
    let now = Instant::now();
    // 简单节流：距上次广播 < min_interval 且非 force 则跳过
    thread_local! {
        static LAST: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    }
    LAST.with(|cell| {
        if let Some(t) = cell.get() {
            if !force && now.duration_since(t) < min_interval {
                return;
            }
        }
        cell.set(Some(now));
        let snap = shared.lock().clone();
        let _ = app.emit("player:state", snap);
    });
}
