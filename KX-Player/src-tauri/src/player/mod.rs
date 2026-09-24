//! 播放内核：libmpv2 适配层。mpv 是播放状态的唯一事实源；
//! Rust 只转发命令、订阅属性并归一化成 `player:state` / `player:ended` 等事件。
//!
//! 所有 mpv 访问（写属性、发命令、读属性）都经 [`queue::CommandQueue`] 投给**唯一一条**
//! 命令线程按序执行（P1-2）：`libmpv` 的调用本身线程安全，但两个线程同时下发就没人保证
//! 先后，而 mpv 只按收到的顺序执行。

pub mod embed;
pub mod queue;

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::PlayerState;
use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, WebviewWindow};

pub struct PlayerCore {
    pub app: AppHandle,
    /// 唯一的 mpv 命令线程（`None` = 还没起来）。它同时是「mpv 是否就绪」的**唯一判据** ——
    /// 队列持有那份 `Arc<Mpv>`，别处不再各存一份：初始化失败要能整份回滚，
    /// 而「一半就绪」的内核最坏（命令发不出去，读属性却以为自己能读）。
    cmd: Mutex<Option<Arc<queue::CommandQueue<Arc<Mpv>>>>>,
    start_lock: Mutex<()>,
    pub shared: Arc<Mutex<PlayerState>>,
    last_emit_ms: Arc<AtomicU64>,
    /// 最后已知的舞台矩形（主窗口移动/缩放/聚焦时用于重放）
    last_rect: parking_lot::Mutex<Option<embed::StageRect>>,
    /// 覆盖窗口当前是否挂靠在独立悬浮窗（pip）上
    pip_active: AtomicBool,
    /// 已下发但尚未开始的加载代际（0 = 没有）。见 [`apply_property`]：
    /// 起播命令与 mpv 的 `StartFile` 之间，事件线程收到的文件级属性全是上一首的。
    pending_load: Arc<AtomicU64>,
}

/// 属于「某一条曲目」的属性：换曲时它们必须整批换掉。
/// 音量/静音/倍速是**设置**，跨曲目有效，因此不在此列。
fn is_file_scoped_property(name: &str) -> bool {
    matches!(name, "time-pos" | "duration" | "video-params/dw" | "video-params/dh")
}

/// 把一次属性观察并进共享状态，返回「值变了、值得强推一次」。
///
/// `pending`（有事在加载）期间，文件级属性一律丢弃：前端发出 `player_play` 时
/// 已经把 currentId 换成新曲目，此刻放行的旧 `time-pos`/`duration` 会被它当成
/// 新曲目的进度（P0-5：旧请求的迟到属性不许改当前代）。
///
/// 判据只看「命令是否已下发、新文件是否已开始」：mpv 的事件队列是 FIFO，
/// 旧文件的属性一定在它的 `EndFile` 之前、新文件的 `StartFile` 之前生成，
/// 因此 `StartFile` 一到，后面收到的文件级属性必然属于新文件，不必再挡。
fn apply_property(s: &mut PlayerState, name: &str, change: &PropertyData, pending: bool) -> bool {
    if pending && is_file_scoped_property(name) {
        return false;
    }
    match change {
        PropertyData::Double(v) if name == "time-pos" => {
            s.position = v.max(0.0);
        }
        PropertyData::Double(v) if name == "duration" && (s.duration - v).abs() > 0.01 => {
            s.duration = v.max(0.0);
            return true;
        }
        PropertyData::Flag(p) if name == "pause" && s.playing == *p => {
            s.playing = !*p;
            return true;
        }
        PropertyData::Double(v) if name == "speed" && (s.speed - v).abs() > 0.001 => {
            s.speed = *v;
            return true;
        }
        PropertyData::Double(v) if name == "volume" => {
            s.volume = v.clamp(0.0, 100.0);
        }
        PropertyData::Flag(m) if name == "mute" && s.muted != *m => {
            s.muted = *m;
            return true;
        }
        PropertyData::Double(v) if name == "video-params/dw" && (s.video_w - v).abs() > 0.5 => {
            s.video_w = *v;
            return true;
        }
        PropertyData::Double(v) if name == "video-params/dh" && (s.video_h - v).abs() > 0.5 => {
            s.video_h = *v;
            return true;
        }
        _ => {}
    }
    false
}

/// mpv 的 `sub-back-color` 要 `#AARRGGBB`，而 `<input type="color">` 只给 `#RRGGBB` ——
/// 不透明度在这里合成。0% = `#00000000`，即 mpv 默认的「无背景条」。
///
/// 只吃 ASCII 十六进制字符：`#中文` 这类输入会被过滤成非法长度并退到黑底，
/// 而不是在 `&hex[..6]` 上按字节切出 panic（本仓已经踩过一次同类坑）。
fn sub_back_color_value(rgb: &str, opacity_percent: f64) -> String {
    // 顺手规范成大写：`<input type="color">` 给的是小写，而「同一个颜色两种写法」
    // 会让属性值比对与日志看起来像变了色。
    let hex: String = rgb
        .trim()
        .trim_start_matches('#')
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(6)
        .collect::<String>()
        .to_ascii_uppercase();
    let rgb6 = if hex.len() == 6 { hex } else { "000000".to_string() };
    let alpha = (opacity_percent.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u32;
    format!("#{alpha:02X}{rgb6}")
}

impl PlayerCore {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            cmd: Mutex::new(None),
            start_lock: Mutex::new(()),
            shared: Arc::new(Mutex::new(PlayerState {
                volume: 85.0,
                ..Default::default()
            })),
            last_emit_ms: Arc::new(AtomicU64::new(0)),
            last_rect: parking_lot::Mutex::new(None),
            pip_active: AtomicBool::new(false),
            pending_load: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 命令队列句柄。没初始化（或初始化失败）时与「mpv 还没起来」是同一件事：
    /// 命令线程是 mpv 的附属品，两者要么都在，要么都不在。
    fn queue(&self) -> IpcResult<Arc<queue::CommandQueue<Arc<Mpv>>>> {
        self.cmd.lock().clone().ok_or_else(IpcError::mpv_not_started)
    }

    fn emit_state(&self, force: bool) {
        emit_state(&self.app, &self.shared, &self.last_emit_ms, force);
    }

    /// 首次播放时初始化 mpv、宿主窗口与命令线程（幂等）。
    ///
    /// 三步都可能失败，而失败**必须可回滚**：宿主窗口线程先于 mpv 建起来，若 mpv 或命令线程
    /// 起不来就把它留着，下次重试会再建一个 —— 两个覆盖窗口叠在同一个舞台上，谁也说不清
    /// 画面归谁。所以每条失败路径都先把已经建好的那部分拆掉，再把「已就绪」判据留在未就绪。
    pub fn ensure_started(&self, window: &WebviewWindow) -> IpcResult<()> {
        if self.cmd.lock().is_some() {
            return Ok(());
        }
        let _g = self.start_lock.lock();
        if self.cmd.lock().is_some() {
            return Ok(());
        }
        let main_hwnd: isize = window
            .hwnd()
            .map_err(|e| IpcError::with_detail(AppErrorCode::MpvEmbedFailed, "获取窗口句柄失败", e))?
            .0 as isize;
        // 舞台线程：创建覆盖窗口并跑消息循环（窗口操作只在该线程执行，避免跨线程 SendMessage 死锁）
        let host = embed::start_stage_thread(main_hwnd)
            .ok_or_else(|| IpcError::new(AppErrorCode::MpvEmbedFailed, "创建 mpv 宿主窗口失败"))?;
        let mpv = match Mpv::with_initializer(|init| {
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
        }) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                embed::destroy();
                return Err(IpcError::with_detail(AppErrorCode::MpvInitFailed, "mpv 初始化失败", e));
            }
        };
        let queue = match queue::CommandQueue::start(Arc::clone(&mpv)) {
            Ok(q) => q,
            Err(e) => {
                embed::destroy();
                return Err(e);
            }
        };

        // 竞态补偿：前端可能在 mpv 初始化完成前就发来舞台矩形（那时 host 还是 None），
        // 此时补应用一次，否则画面窗口会一直停在初始化时的离屏位置（表现为黑屏）。
        self.reposition();

        // 事件线程：属性观察 → 归一化 player:state。它自己不下发 mpv 命令，
        // 需要改属性时（FileLoaded 复位 start）也投给命令线程排队。
        //
        // 顺序上必须先起它、再公开 `cmd`：观察属性/开事件都由它独占完成，
        // 而「公开 cmd」= 允许用户命令进来。反过来的话，`observe_property` 可能落在
        // 第一条 loadfile 之后，那一代的属性就整段漏收（进度条与画面尺寸永远不动）。
        let app = self.app.clone();
        let shared = Arc::clone(&self.shared);
        let pending = Arc::clone(&self.pending_load);
        let last_emit = Arc::clone(&self.last_emit_ms);
        let q = Arc::clone(&queue);
        let spawned = std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn(move || event_loop(mpv, app, shared, pending, last_emit, q));
        if let Err(e) = spawned {
            // 事件线程起不来 = 没有任何属性会被归一化，这个内核等于哑的：整份初始化作废，
            // 下次 play 从干净状态重来（mpv 随队列一起释放，宿主窗口一并拆掉）。
            queue.shutdown();
            embed::destroy();
            return Err(IpcError::with_detail(AppErrorCode::MpvInitFailed, "mpv 事件线程启动失败", e));
        }
        *self.cmd.lock() = Some(queue);
        crate::paths::append_log("[mpv] initialized with embedded window");
        Ok(())
    }

    /// 起播一条曲目。`generation` 由前端每次加载自增（P0-5），随状态广播回传，
    /// 前端据此丢弃上一代的迟到状态。
    ///
    /// 立牌与乐观状态在**调用方**就地写，不等命令线程：事件线程此刻还在放上一首的属性，
    /// 晚一步立牌就等于让它们漏进新代。碰 mpv 的那一段整段投给命令线程，失败回滚也留在作业里。
    #[allow(clippy::too_many_arguments)]
    pub fn play_file(&self, path: &str, resume_ms: f64, vol: f64, muted: bool, speed: f64, video_active: bool, generation: u64) -> IpcResult<()> {
        let q = self.queue()?;
        // 先立牌再动状态：从这一刻到 mpv 送出 StartFile 之间收到的文件级属性都是上一首的。
        // 取 max 是为了让「更新的代际已经接手」时旧值不至于把牌子降回去。
        self.pending_load.fetch_max(generation, Ordering::SeqCst);
        {
            let mut s = self.shared.lock();
            s.generation = s.generation.max(generation);
            s.playing = true;
            s.position = 0.0;
            s.duration = 0.0;
            s.track_path = Some(path.to_string());
            s.video_active = video_active;
        }
        let shared = Arc::clone(&self.shared);
        let pending = Arc::clone(&self.pending_load);
        let file = path.to_string();
        let result = q.submit(None, move |mpv| {
            let result = (|| -> IpcResult<()> {
                mpv.set_property("volume", vol * 100.0).map_err(mpv_err)?;
                mpv.set_property("mute", muted).map_err(mpv_err)?;
                mpv.set_property("speed", speed).map_err(mpv_err)?;
                if resume_ms > 0.0 {
                    mpv.set_property("start", resume_ms / 1000.0).map_err(mpv_err)?;
                } else {
                    // 显式清零：`start` 是上一次续播留下的，不覆盖就会拿旧起点开新曲。
                    // 清零失败不值得让整首起播失败（本次本来就要从 0 开始）。
                    let _ = mpv.set_property("start", 0.0_f64);
                }
                // keep-open=yes 让 mpv 播完后停在「已暂停」，而 pause 属性会跨 loadfile 保留：
                // 不显式清掉，之后每次起播都冻在首帧（进度不动、画面静止，前端一直显示加载中）。
                mpv.set_property("pause", false).map_err(mpv_err)?;
                mpv.command("loadfile", &[file.as_str()]).map_err(mpv_err)
            })();
            if result.is_err() {
                // 这一首根本没起播：把立牌和「正在播放」的假象一起收回。
                // CAS 而不是直接清零——更新的代际若已接手，牌子是它的。
                if pending.compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    let mut s = shared.lock();
                    s.playing = false;
                    s.track_path = None;
                    s.position = 0.0;
                    s.duration = 0.0;
                }
                crate::paths::append_log(&format!("[mpv] 起播失败（generation {generation}）：{file}"));
            }
            result
        });
        self.emit_state(true);
        result
    }

    pub fn set_paused(&self, paused: bool) -> IpcResult<()> {
        self.queue()?.submit(Some("pause"), move |mpv| mpv.set_property("pause", paused).map_err(mpv_err))
    }

    pub fn seek(&self, sec: f64) -> IpcResult<()> {
        self.queue()?.submit(None, move |mpv| mpv.command("seek", &[&format!("{sec:.3}"), "absolute"]).map_err(mpv_err))
    }

    pub fn set_volume(&self, v: f64) -> IpcResult<()> {
        self.queue()?.submit(Some("volume"), move |mpv| mpv.set_property("volume", v * 100.0).map_err(mpv_err))
    }

    pub fn set_muted(&self, m: bool) -> IpcResult<()> {
        self.queue()?.submit(Some("mute"), move |mpv| mpv.set_property("mute", m).map_err(mpv_err))
    }

    pub fn set_speed(&self, s: f64) -> IpcResult<()> {
        self.queue()?.submit(Some("speed"), move |mpv| mpv.set_property("speed", s).map_err(mpv_err))
    }

    pub fn stop(&self) -> IpcResult<()> {
        self.queue()?.submit(None, |mpv| mpv.command("stop", &[]).map_err(mpv_err))?;
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
        // 读取也排进命令线程：一边 loadfile 一边读 track-list，拿到的是半截状态。
        let q = match self.queue() {
            Ok(q) => q,
            Err(_) => return Vec::new(),
        };
        // track-list 是节点数组，字符串化后不易解析；改用 sid 探测 + 逐个取属性更稳，
        // 这里用 mpv 的 "track-list/count" + "/N/id|title|lang" 属性路径读取。
        q.query(|mpv| {
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
        })
        .unwrap_or_default()
    }

    /// 选择字幕轨（id<=0 表示关闭字幕）
    pub fn set_subtitle_track(&self, id: i64) -> IpcResult<()> {
        // sid 与 sub-visibility 是一次选择的两个面，同一条作业里下发，不许交错
        self.queue()?.submit(Some("sid"), move |mpv| {
            let v = if id > 0 { id.to_string() } else { "no".to_string() };
            mpv.set_property("sid", v).map_err(mpv_err)?;
            // 记住选择（mpv 会持久化 sub-visibility）
            mpv.set_property("sub-visibility", id > 0).map_err(mpv_err)?;
            Ok(())
        })
    }

    /// 字幕开关（不改变当前轨）
    pub fn set_subtitle_visible(&self, visible: bool) -> IpcResult<()> {
        self.queue()?.submit(Some("sub-visibility"), move |mpv| mpv.set_property("sub-visibility", visible).map_err(mpv_err))
    }

    /// 字幕时间偏移（秒，正=延后显示）
    pub fn set_subtitle_delay(&self, sec: f64) -> IpcResult<()> {
        self.queue()?.submit(Some("sub-delay"), move |mpv| mpv.set_property("sub-delay", sec).map_err(mpv_err))
    }

    /// 字幕样式（mpv sub-* 属性遥控）
    pub fn set_sub_style(&self, style: crate::model::SubStyle) -> IpcResult<()> {
        // 整份样式一条作业：分开发会被「拖滑块」的中间态插进来，落到 mpv 上是一份混合样式
        self.queue()?.submit(Some("sub-style"), move |mpv| {
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
            if let Some(v) = style.back_color {
                // 0% 也要下发：用户把不透明度拖回 0 时得真的回到「无背景条」
                let value = sub_back_color_value(&v, style.back_opacity.unwrap_or(0.0));
                mpv.set_property("sub-back-color", value).map_err(mpv_err)?;
            }
            if let Some(v) = style.ass_override {
                mpv.set_property("sub-ass-override", v).map_err(mpv_err)?;
            }
            Ok(())
        })
    }

    /// 响度均衡：volume-gain 叠加在用户音量之上；无分析值时必须清零上一首的增益。
    pub fn apply_loudness_gain(&self, track_lufs: Option<f64>, target_lufs: f64) -> IpcResult<()> {
        self.queue()?.submit(Some("volume-gain"), move |mpv| {
            mpv.set_property("volume-gain", loudness_gain_db(track_lufs, target_lufs))
                .map_err(mpv_err)
        })
    }

    /// 「仅音频」= vid=no（引擎始终是 mpv，播放位置不丢）
    pub fn set_video_enabled(&self, enabled: bool) -> IpcResult<()> {
        let shared = Arc::clone(&self.shared);
        let r = self.queue()?.submit(Some("vid"), move |mpv| {
            mpv.set_property("vid", if enabled { "auto".to_string() } else { "no".to_string() })
                .map_err(mpv_err)?;
            shared.lock().video_active = enabled;
            Ok(())
        });
        self.emit_state(true);
        r
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

    /// 销毁覆盖窗口（应用退出时）。顺带关停命令队列：队列一死，退场过程中再来的
    /// `player_*` 命令会当场拿到「命令线程已退出」，而不是排进一条即将消失的线程。
    pub fn destroy_host(&self) {
        if let Some(q) = self.cmd.lock().clone() {
            // 先让 mpv 收 quit：事件线程收到 Shutdown 后自己退出，不必等进程收尸
            let _ = q.post(None, |mpv| {
                let _ = mpv.command("quit", &[]);
                Ok(())
            });
            q.shutdown();
        }
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
        let q = match self.queue() {
            Ok(q) => q,
            Err(_) => return vec![default_device()],
        };
        q.query(|mpv| {
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
        })
        .unwrap_or_else(|_| vec![default_device()])
    }

    pub fn set_device(&self, id: &str) -> IpcResult<()> {
        let id = id.to_string();
        self.queue()?.submit(Some("audio-device"), move |mpv| mpv.set_property("audio-device", id).map_err(mpv_err))
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

fn event_loop(
    mpv: Arc<Mpv>,
    app: AppHandle,
    shared: Arc<Mutex<PlayerState>>,
    pending: Arc<AtomicU64>,
    last_emit: Arc<AtomicU64>,
    q: Arc<queue::CommandQueue<Arc<Mpv>>>,
) {
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
                let is_pending = pending.load(Ordering::SeqCst) != 0;
                let force = {
                    let mut s = shared.lock();
                    apply_property(&mut s, name, &change, is_pending)
                };
                emit_state(&app, &shared, &last_emit, force);
            }
            Some(Ok(Event::StartFile)) => {
                // 新文件已开始：这一代从「加载中」变成「在播」，牌子收回，
                // 之后收到的文件级属性必然属于它（mpv 事件队列是 FIFO）。
                let generation = {
                    pending.swap(0, Ordering::SeqCst);
                    let mut s = shared.lock();
                    s.playing = true;
                    s.position = 0.0;
                    // 换曲即失配：不清掉的话悬浮窗会按上一首的比例开窗，新画面又要黑边补差
                    s.video_w = 0.0;
                    s.video_h = 0.0;
                    s.generation
                };
                let _ = app.emit("player:loading", serde_json::json!({ "generation": generation }));
                emit_state(&app, &shared, &last_emit, true);
            }
            Some(Ok(Event::FileLoaded)) => {
                // 用 start 属性实现的续播已生效；复位以免影响下一次加载。
                // 但此刻若又有起播在途（连点两首），这条复位会插到那一首的 loadfile 之前，
                // 把它的续播起点抹掉——有牌子在就先不复位，交给它自己的 FileLoaded。
                if pending.load(Ordering::SeqCst) == 0 {
                    let _ = q.post(Some("start"), |mpv| mpv.set_property("start", 0.0_f64).map_err(mpv_err));
                }
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
                // 还没走到 StartFile 就结束了（加载失败 / 被 stop）：牌子顺手收回，
                // 免得共享状态一直冻在「加载中」。
                let pending_gen = pending.swap(0, Ordering::SeqCst);
                let generation = if pending_gen != 0 { pending_gen } else { shared.lock().generation };
                let _ = app.emit(
                    "player:ended",
                    serde_json::json!({ "reason": payload, "generation": generation }),
                );
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
                emit_state(&app, &shared, &last_emit, false);
            }
        }
    }
}

/// 共享状态的节流广播（命令线程与事件线程共用）。
///
/// 节流时钟是**进程级**的（`last_ms`）：两条线程都往同一条 `player:state` 上写，
/// 各记各的时钟等于没节流 —— 一边刚推完，另一边立刻又推一份同样的快照。
/// `force` 表示「值确实变了」，必须立刻推，不等节流窗口。
fn emit_state(app: &AppHandle, shared: &Arc<Mutex<PlayerState>>, last_ms: &AtomicU64, force: bool) {
    let now = now_ms();
    if !force && now.saturating_sub(last_ms.load(Ordering::SeqCst)) < 100 {
        return;
    }
    last_ms.store(now, Ordering::SeqCst);
    let snap = shared.lock().clone();
    let _ = app.emit("player:state", snap);
}

#[cfg(test)]
mod tests {
    use super::{apply_property, ensure_default_device, is_file_scoped_property, loudness_gain_db, sub_back_color_value};
    use crate::model::{AudioDevice, PlayerState};
    use libmpv2::events::PropertyData;

    fn dev(id: &str, label: &str) -> AudioDevice {
        AudioDevice { device_id: id.into(), label: label.into() }
    }

    /// P0-5：起播命令已下发、mpv 还没报出 StartFile 的这一段里，事件线程收到的
    /// 文件级属性全是上一首的。前端此刻已经把 currentId 换成了新曲目，放行就是
    /// 「新曲目 + 旧进度」——进度条跳一下，写盘的进度还落到新曲目名下。
    #[test]
    fn a_pending_load_drops_the_previous_tracks_file_properties() {
        let mut s = PlayerState { duration: 301.0, ..Default::default() };
        for name in ["time-pos", "duration", "video-params/dw", "video-params/dh"] {
            assert!(is_file_scoped_property(name), "{name} 是文件级属性");
        }
        assert!(!is_file_scoped_property("volume"), "音量是设置，不是文件属性");
        for (name, change) in [
            ("time-pos", PropertyData::Double(128.5)),
            ("duration", PropertyData::Double(302.0)),
            ("video-params/dw", PropertyData::Double(1920.0)),
            ("video-params/dh", PropertyData::Double(1080.0)),
        ] {
            assert!(!apply_property(&mut s, name, &change, true), "{name} 不该在加载期间进状态");
        }
        assert_eq!(
            (s.position, s.duration, s.video_w, s.video_h),
            (0.0, 301.0, 0.0, 0.0),
            "上一首的值一个都不许漏进来"
        );
        // StartFile 之后牌子收回，同一批属性照单收下
        assert!(apply_property(&mut s, "duration", &PropertyData::Double(302.0), false));
        assert!(apply_property(&mut s, "video-params/dw", &PropertyData::Double(1920.0), false));
        assert_eq!((s.duration, s.video_w), (302.0, 1920.0));
    }

    /// 加载期间被挡的是「某一条曲目的属性」；音量/静音/倍速/暂停是跨曲目的传输状态，
    /// 挡下来会让用户在新曲目开始前调的音量被无声吞掉。
    #[test]
    fn a_pending_load_still_applies_transport_settings() {
        let mut s = PlayerState { playing: true, ..Default::default() };
        assert!(apply_property(&mut s, "pause", &PropertyData::Flag(true), true));
        assert!(!s.playing, "pause 属性取反后写回 playing");
        assert!(apply_property(&mut s, "mute", &PropertyData::Flag(true), true));
        assert!(s.muted);
        assert!(apply_property(&mut s, "speed", &PropertyData::Double(1.5), true));
        assert_eq!(s.speed, 1.5);
        assert!(!apply_property(&mut s, "volume", &PropertyData::Double(70.0), true));
        assert_eq!(s.volume, 70.0, "音量照收，只是不值得为它强推一次状态");
    }

    #[test]
    fn unknown_properties_do_not_touch_the_state() {
        let mut s = PlayerState { position: 12.0, ..Default::default() };
        assert!(!apply_property(&mut s, "sub-visibility", &PropertyData::Flag(false), false));
        assert_eq!(s.position, 12.0);
        assert_eq!(s.volume, 0.0);
    }

    /// P0-24：UI 给的是 `#RRGGBB` + 不透明度，mpv 要 `#AARRGGBB`。
    #[test]
    fn sub_back_color_composes_alpha_in_front() {
        assert_eq!(sub_back_color_value("#000000", 0.0), "#00000000", "0% = mpv 默认的无背景条");
        assert_eq!(sub_back_color_value("#000000", 50.0), "#80000000");
        assert_eq!(sub_back_color_value("#ffffff", 100.0), "#FFFFFFFF");
        assert_eq!(sub_back_color_value("#FFCC00", 30.0), "#4DFFCC00");
        assert_eq!(sub_back_color_value("  #12ab34  ", 100.0), "#FF12AB34", "trim + 大小写无关");
        assert_eq!(sub_back_color_value("#123", 100.0), "#FF000000", "位数不足退回黑底而不是截断越界");
        assert_eq!(sub_back_color_value("#中文", 50.0), "#80000000", "非十六进制字符不会 panic");
        assert_eq!(sub_back_color_value("#000000", 999.0), "#FF000000", "越界钳到 100%");
        assert_eq!(sub_back_color_value("#000000", -5.0), "#00000000", "越界钳到 0%");
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
