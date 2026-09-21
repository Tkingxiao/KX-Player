//! 播放内核：libmpv2 适配层。mpv 是播放状态的唯一事实源；
//! Rust 只转发命令、订阅属性并归一化成 `player:state` / `player:ended` 等事件。

pub mod embed;

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::PlayerState;
use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// 覆盖窗口当前是否挂靠在独立悬浮窗（pip）上
    pip_active: AtomicBool,
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
            pip_active: AtomicBool::new(false),
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
    pub fn ensure_started(&self, window: &WebviewWindow) -> IpcResult<()> {
        if self.mpv.get().is_some() {
            return Ok(());
        }
        let _g = self.start_lock.lock();
        if self.mpv.get().is_some() {
            return Ok(());
        }
        let main_hwnd: isize = window
            .hwnd()
            .map_err(|e| IpcError::with_detail(AppErrorCode::MpvEmbedFailed, "获取窗口句柄失败", e))?
            .0 as isize;
        // 舞台线程：创建覆盖窗口并跑消息循环（窗口操作只在该线程执行，避免跨线程 SendMessage 死锁）
        let host = embed::start_stage_thread(main_hwnd)
            .ok_or_else(|| IpcError::new(AppErrorCode::MpvEmbedFailed, "创建 mpv 宿主窗口失败"))?;
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
        .map_err(|e| IpcError::with_detail(AppErrorCode::MpvInitFailed, "mpv 初始化失败", e))?;
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
            .map_err(|e| IpcError::with_detail(AppErrorCode::MpvInitFailed, "mpv 事件线程启动失败", e))?;
        crate::paths::append_log("[mpv] initialized with embedded window");
        Ok(())
    }

    pub fn play_file(&self, path: &str, resume_ms: f64, vol: f64, muted: bool, speed: f64, video_active: bool) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
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
        // keep-open=yes 让 mpv 播完后停在「已暂停」，而 pause 属性会跨 loadfile 保留：
        // 不显式清掉，之后每次起播都冻在首帧（进度不动、画面静止，前端一直显示加载中）。
        mpv.set_property("pause", false).map_err(mpv_err)?;
        mpv.command("loadfile", &[path]).map_err(mpv_err)?;
        self.emit_state(true);
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("pause", paused).map_err(mpv_err)
    }

    pub fn seek(&self, sec: f64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.command("seek", &[&format!("{sec:.3}"), "absolute"]).map_err(mpv_err)
    }

    pub fn set_volume(&self, v: f64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("volume", v * 100.0).map_err(mpv_err)
    }

    pub fn set_muted(&self, m: bool) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("mute", m).map_err(mpv_err)
    }

    pub fn set_speed(&self, s: f64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("speed", s).map_err(mpv_err)
    }

    pub fn stop(&self) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.command("stop", &[]).map_err(mpv_err)?;
        {
            let mut s = self.shared.lock();
            s.playing = false;
            s.position = 0.0;
            s.track_path = None;
            s.video_active = false;
            s.video_w = 0.0;
            s.video_h = 0.0;
        }
        self.set_stage_visible(false);
        self.emit_state(true);
        Ok(())
    }

    /// 字幕轨列表（id / 标题 / 语言），mpv 的 track-list 是字典数组
    pub fn subtitle_tracks(&self) -> Vec<crate::model::SubtitleTrack> {
        let mpv = match self.mpv.get() {
            Some(m) => m,
            None => return Vec::new(),
        };
        // track-list 是节点数组，字符串化后不易解析；改用 sid 探测 + 逐个取属性更稳，
        // 这里用 mpv 的 "track-list/count" + "/N/id|title|lang" 属性路径读取。
        let count: i64 = mpv.get_property("track-list/count").unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..count.max(0) {
            let type_: String = match mpv.get_property(&format!("track-list/{i}/type")) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if type_ != "sub" {
                continue;
            }
            let id: i64 = mpv.get_property(&format!("track-list/{i}/id")).unwrap_or(0);
            let title: String = mpv.get_property(&format!("track-list/{i}/title")).unwrap_or_default();
            let lang: String = mpv.get_property(&format!("track-list/{i}/lang")).unwrap_or_default();
            let selected: bool = mpv.get_property(&format!("track-list/{i}/selected")).unwrap_or(false);
            out.push(crate::model::SubtitleTrack {
                id,
                title: if title.is_empty() { format!("字幕 {id}") } else { title },
                lang,
                selected,
            });
        }
        out
    }

    /// 选择字幕轨（id<=0 表示关闭字幕）
    pub fn set_subtitle_track(&self, id: i64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        let v = if id > 0 { id.to_string() } else { "no".to_string() };
        mpv.set_property("sid", v).map_err(mpv_err)?;
        // 记住选择（mpv 会持久化 sub-visibility）
        mpv.set_property("sub-visibility", id > 0).map_err(mpv_err)?;
        Ok(())
    }

    /// 字幕开关（不改变当前轨）
    pub fn set_subtitle_visible(&self, visible: bool) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("sub-visibility", visible).map_err(mpv_err)
    }

    /// 字幕时间偏移（秒，正=延后显示）
    pub fn set_subtitle_delay(&self, sec: f64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("sub-delay", sec).map_err(mpv_err)
    }

    /// 字幕样式（mpv sub-* 属性遥控）
    pub fn set_sub_style(&self, style: crate::model::SubStyle) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        if let Some(v) = style.font {
            mpv.set_property("sub-font", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.font_size {
            mpv.set_property("sub-font-size", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.color {
            mpv.set_property("sub-color", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.border_color {
            mpv.set_property("sub-border-color", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.border_size {
            mpv.set_property("sub-border-size", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.shadow_offset {
            mpv.set_property("sub-shadow-offset", v).map_err(mpv_err)?;
        }
        if let Some(v) = style.pos {
            mpv.set_property("sub-pos", v).map_err(mpv_err)?;
        }
        Ok(())
    }

    /// 响度均衡：volume-gain 叠加在用户音量之上；无分析值时必须清零上一首的增益。
    pub fn apply_loudness_gain(&self, track_lufs: Option<f64>, target_lufs: f64) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("volume-gain", loudness_gain_db(track_lufs, target_lufs))
            .map_err(mpv_err)?;
        Ok(())
    }

    /// 「仅音频」= vid=no（引擎始终是 mpv，播放位置不丢）
    pub fn set_video_enabled(&self, enabled: bool) -> IpcResult<()> {

        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("vid", if enabled { "auto".to_string() } else { "no".to_string() })
            .map_err(mpv_err)?;
        {
            self.shared.lock().video_active = enabled;
        }
        self.emit_state(true);
        Ok(())
    }

    /// 前端下发舞台几何。同一时刻只有一个窗口有资格决定画面在哪：
    /// - 挂在浮窗上时，忽略主窗口的命令。主窗口的失焦、视图切换都会发 `visible:false`，
    ///   它和浮窗的几何命令在同一个 FIFO 队列里竞速，后到者赢 ——
    ///   表现为「不在视频界面打开小窗时，小窗画面消失」。
    /// - 浮窗已关闭/还原时，忽略浮窗的命令。浮窗只是被 hide，它的 WebView 还活着并会继续
    ///   上报几何，而那份矩形是**浮窗客户区**坐标，拿去换算主窗口就把画面甩到左上角。
    pub fn set_stage_rect(&self, x: i32, y: i32, w: i32, h: i32, visible: bool, from_pip: bool) {
        if from_pip != self.pip_active.load(Ordering::SeqCst) {
            return;
        }
        let rect = embed::StageRect { x, y, w, h, visible };
        *self.last_rect.lock() = Some(rect);
        embed::set_stage_rect(rect);
    }

    pub fn set_stage_visible(&self, visible: bool) {
        // 挂靠在悬浮窗上时，主窗口的可见性不应影响覆盖窗口（悬浮窗独立显示）
        if !visible && self.pip_active.load(Ordering::SeqCst) {
            return;
        }
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
    /// 悬浮窗挂靠期间不受主窗口焦点影响（浮窗独立于主窗口存在）。
    pub fn focus_changed(&self, focused: bool) {
        if self.pip_active.load(Ordering::SeqCst) {
            return;
        }
        if focused {
            if let Some(rect) = *self.last_rect.lock() {
                embed::set_stage_rect(rect);
            }
        } else {
            embed::set_os_visible_cmd(false);
        }
    }

    /// 切换覆盖窗口挂靠目标：Some(hwnd)=独立悬浮窗（pip），None=挂回主窗口。
    /// 清空 last_rect：坐标系已切换，旧矩形在新坐标系下完全无意义；
    /// 前端（PipRoot 或 StageView）在挂靠完成后会重新下发正确几何。
    pub fn attach_overlay(&self, target: Option<isize>) {
        self.pip_active.store(target.is_some(), Ordering::SeqCst);
        *self.last_rect.lock() = None;   // 必须清空，防止 reposition() 用错坐标系
        embed::attach(target);
    }

    /// 提前声明「画面即将由悬浮窗接管」。pip_open 在 show() 之前调用：
    /// 显示浮窗会让主窗口失焦，那次 Focused(false) 若早于 attach_overlay 到达，
    /// 就会把刚挂好底的视频层一起藏掉（小窗只剩声音）。
    pub fn mark_pip_active(&self, active: bool) {
        self.pip_active.store(active, Ordering::SeqCst);
    }

    /// 分离覆盖窗口（pip 关闭 / 还原时专用）：
    /// 先强制隐藏覆盖窗口（避免在 Attach(None) 被 kx-stage 处理前短暂露出错误位置），
    /// 再挂回主窗口并清空 last_rect。
    pub fn detach_overlay(&self) {
        self.pip_active.store(false, Ordering::SeqCst);
        // 先发 Visible(false) 入队，让 kx-stage 先隐藏再切 owner
        embed::set_os_visible_cmd(false);
        *self.last_rect.lock() = None;
        embed::attach(None);
    }

    pub fn is_pip_active(&self) -> bool {
        self.pip_active.load(Ordering::SeqCst)
    }

    /// 销毁覆盖窗口（应用退出时）
    pub fn destroy_host(&self) {
        embed::destroy();
    }

    /// 输出设备列表访问器（commands/player.rs 的 player_list_devices 调用）。
    /// 每次调用都重新读取 mpv：mpv 是首次播放时才懒加载的（见 ensure_started），
    /// 启动期缓存只会拿到单条兜底项，因此这里不做缓存。
    pub fn list_devices(&self) -> Vec<crate::model::AudioDevice> {
        self.refresh_devices()
    }

    /// 重新枚举系统输出设备。mpv 的 audio-device-list 是节点数组，
    /// 字符串化后按 `,` / `/` 切分会把多设备列表解析错（描述文本本身可能含逗号或斜杠），
    /// 因此改用 "audio-device-list/count" + "/N/name|description" 属性路径逐个读取，
    /// 与 subtitle_tracks() 读 track-list 的方式一致。
    pub fn refresh_devices(&self) -> Vec<crate::model::AudioDevice> {
        let mpv = match self.mpv.get() {
            Some(m) => m,
            None => return vec![default_device()],
        };
        let count: i64 = mpv.get_property("audio-device-list/count").unwrap_or(0);
        let mut devices: Vec<crate::model::AudioDevice> = Vec::new();
        for i in 0..count.max(0) {
            let name: String = mpv.get_property(&format!("audio-device-list/{i}/name")).unwrap_or_default();
            let name = name.trim().to_string();
            if name.is_empty() || devices.iter().any(|d| d.device_id == name) {
                continue;
            }
            let desc: String = match mpv.get_property::<String>(&format!("audio-device-list/{i}/description")) {
                Ok(v) => v.trim().to_string(),
                Err(_) => String::new(),
            };
            let label = if desc.is_empty() { name.clone() } else { desc };
            devices.push(crate::model::AudioDevice { device_id: name, label });
        }
        // 始终保留一条「系统默认」入口；mpv 自身已返回 auto 条目不重复添加。
        ensure_default_device(devices)
    }

    pub fn set_device(&self, id: &str) -> IpcResult<()> {
        let mpv = self.mpv.get().ok_or_else(IpcError::mpv_not_started)?;
        mpv.set_property("audio-device", id.to_string()).map_err(mpv_err)
    }

    pub fn snapshot(&self) -> PlayerState {
        self.shared.lock().clone()
    }
}

fn loudness_gain_db(track_lufs: Option<f64>, target_lufs: f64) -> f64 {
    track_lufs.map(|lufs| (target_lufs - lufs).clamp(-12.0, 12.0)).unwrap_or(0.0)
}

fn default_device() -> crate::model::AudioDevice {
    crate::model::AudioDevice { device_id: "auto".into(), label: "系统默认输出".into() }
}

/// 归一化输出设备列表：保证存在且仅存在一条 device_id == "auto" 的入口，并置于首位。
/// 纯函数（不依赖 mpv），便于单元测试；供 list_devices/refresh_devices 复用。
fn ensure_default_device(mut devices: Vec<crate::model::AudioDevice>) -> Vec<crate::model::AudioDevice> {
    match devices.iter().position(|d| d.device_id == "auto") {
        Some(pos) => {
            let mut auto = devices.remove(pos);
            if auto.label.trim().is_empty() {
                auto.label = default_device().label;
            }
            devices.insert(0, auto);
        }
        None => devices.insert(0, default_device()),
    }
    devices
}

/// mpv 的 `set_property`/`command` 失败既不是初始化失败也不是嵌入失败，码取 `Internal`；
/// 原始 `libmpv2::Error` 文本进 `detail`，排查时仍能看到。
fn mpv_err(e: libmpv2::Error) -> IpcError {
    IpcError::with_detail(AppErrorCode::Internal, "播放器指令失败", e)
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
    // 画面显示尺寸（含 PAR）：悬浮窗据此把窗口锁成同一比例，避免左右黑边
    let _ = mpv.observe_property("video-params/dw", Format::Double, 7);
    let _ = mpv.observe_property("video-params/dh", Format::Double, 8);
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
                        PropertyData::Double(v) if name == "video-params/dw" => {
                            if (s.video_w - v).abs() > 0.5 {
                                s.video_w = v;
                                force = true;
                            }
                        }
                        PropertyData::Double(v) if name == "video-params/dh"
                            && (s.video_h - v).abs() > 0.5 => {
                                s.video_h = v;
                                force = true;
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
                    // 换曲即失配：不清掉的话悬浮窗会按上一首的比例开窗，新画面又要黑边补差
                    s.video_w = 0.0;
                    s.video_h = 0.0;
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

#[cfg(test)]
mod tests {
    use super::{ensure_default_device, loudness_gain_db};
    use crate::model::AudioDevice;

    fn dev(id: &str, label: &str) -> AudioDevice {
        AudioDevice { device_id: id.into(), label: label.into() }
    }

    #[test]
    fn loudness_gain_is_zero_without_measurement() {
        assert_eq!(loudness_gain_db(None, -23.0), 0.0);
    }

    #[test]
    fn loudness_gain_uses_target_difference_and_clamps() {
        assert_eq!(loudness_gain_db(Some(-28.0), -23.0), 5.0);
        assert_eq!(loudness_gain_db(Some(-50.0), -23.0), 12.0);
        assert_eq!(loudness_gain_db(Some(-5.0), -23.0), -12.0);
    }

    #[test]
    fn device_list_keeps_real_devices_and_prepends_default() {
        let out = ensure_default_device(vec![dev("wasapi/{abc}", "扬声器 (Realtek)")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].device_id, "auto");
        assert_eq!(out[0].label, "系统默认输出");
        assert_eq!(out[1].device_id, "wasapi/{abc}");
        assert_eq!(out[1].label, "扬声器 (Realtek)");
    }

    #[test]
    fn device_list_does_not_duplicate_mpv_auto_entry() {
        let out = ensure_default_device(vec![dev("auto", "Autoselect device"), dev("null", "Null")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].device_id, "auto");
        assert_eq!(out[0].label, "Autoselect device");
        assert_eq!(out[1].device_id, "null");
    }

    #[test]
    fn device_list_falls_back_to_chinese_label_for_empty_auto_description() {
        let out = ensure_default_device(vec![dev("auto", "   ")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "系统默认输出");
    }

    #[test]
    fn empty_device_list_still_yields_default() {
        let out = ensure_default_device(Vec::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].device_id, "auto");
    }
}
