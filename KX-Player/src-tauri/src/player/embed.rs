//! mpv 嵌入窗口（Windows）：独立无边框「覆盖窗口」承载 libmpv 的视频渲染，
//! 覆盖在主窗口的舞台矩形上。
//!
//! ## 为什么必须是「专用 UI 线程 + 消息循环」
//! 覆盖窗口是顶层 WS_POPUP 窗口。对它做 SetWindowPos/ShowWindow 时，系统需要向该窗口
//! 所属线程同步 SendMessage（WM_WINDOWPOSCHANGING 等）；如果该线程不泵消息（例如 tokio
//! 运行时线程），这次调用将永久阻塞 —— 表现为「视频一播放窗口就冻结」且舞台矩形永远不生效
//! （黑屏）。因此本模块：① 窗口由专用线程创建（线程即窗口所有者）；② 该线程跑标准消息循环；
//! ③ 所有窗口操作都先入队，再 PostMessage 唤醒该线程，在它自己的线程上执行。
//!
//! ## 两种层序：覆盖（主窗口）与垫底（悬浮窗）
//! 主窗口场景用「覆盖」：宿主窗口是顶层窗口，直接压在 WebView 之上（WebView2 用
//! DirectComposition 呈现，同一顶层窗口内任何原生子窗口都在其合成内容之下，压不穿）。
//!
//! 悬浮窗（pip）场景反过来，用「垫底」：pip 窗口本身是 transparent 的顶层窗口，把宿主
//! 窗口放在它**下面**，视频就从 pip 的透明像素里透出来 —— 于是 DOM 控件（标题、按钮、
//! 进度条）天然浮在画面之上，鼠标消息也正常落到 pip 窗口，不需要再靠轮询光标猜 hover。
//! 垫底时宿主窗口不能以 pip 为 owner（owned 窗口恒在其 owner 之上），所以 owner 恒为
//! 主窗口，靠显式的 z 序断言维持「宿主在下、pip 在上」这一对关系（见 raise_above_host）。
//!
//! ## 交互
//! wndproc 对 WM_NCHITTEST 返回 HTTRANSPARENT：鼠标事件落到下方同线程的主窗口
//! （WebView 继续处理双击全屏、控制条等）；配合 WS_EX_NOACTIVATE / MA_NOACTIVATE
//! 保证永不抢焦点。

#![cfg(windows)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, GetStockObject, BLACK_BRUSH, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_NOTOPMOST,
    HWND_TOPMOST, PostMessageW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, GWLP_HWNDPARENT, HTTRANSPARENT, MA_NOACTIVATE, MSG, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, WM_DESTROY, WM_ERASEBKGND, WM_MOUSEACTIVATE,
    WM_NCHITTEST, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

const HOST_CLASS: &str = "kx_mpv_host";
/// 私有消息：通知舞台线程有新的窗口命令
const WM_APP_STAGE: u32 = 0x8001;

/// 舞台矩形（客户区物理像素）
#[derive(Debug, Clone, Copy)]
pub struct StageRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy)]
enum StageCmd {
    Rect(StageRect),
    Visible(bool),
    /// 切换覆盖窗口的挂靠目标：Some(hwnd)=独立悬浮窗（pip），None=主窗口。
    /// 同时把覆盖窗口的 owner 切到目标窗口，保证 z 序在目标之上、且随目标最小化而隐藏。
    Attach(Option<isize>),
    Destroy,
}

/// 离散命令的排队上限。`Rect` 洪泛已被 [`push_cmd`] 合并挡在前面，这里是最后一道保险：
/// 舞台线程真卡住时，队列不许无声涨到吃光内存。
const MAX_PENDING_CMDS: usize = 64;

static CMDS: Mutex<VecDeque<StageCmd>> = Mutex::new(VecDeque::new());
/// 当前一代舞台线程的宿主窗口（`enqueue` 靠它唤醒该线程）
static HOST: Mutex<Option<isize>> = Mutex::new(None);
static MAIN_HWND: Mutex<Option<isize>> = Mutex::new(None);
/// 当前挂靠目标窗口（None=主窗口）。坐标换算（ClientToScreen）以目标窗口的客户区为基准。
static TARGET: Mutex<Option<isize>> = Mutex::new(None);
/// 上一代舞台线程的退出信号。舞台线程是一次性的：`destroy()` 之后它处理完 Destroy 就退出，
/// 下一次初始化另起一代 —— 换代前必须先确认上一代真的不在了（见 [`start_stage_thread`]）。
static EXIT_RX: Mutex<Option<std::sync::mpsc::Receiver<()>>> = Mutex::new(None);

thread_local! {
    /// 本线程创建的那扇宿主窗口。命令只应用到它，不读全局 `HOST` ——
    /// 换代时全局会先被新的一代覆盖，而旧线程还在把队列抽干净。
    static MY_HOST: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn host_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 点击穿透：鼠标交给下方同线程的主窗口（WebView 继续处理交互）
        WM_NCHITTEST => return HTTRANSPARENT as LRESULT,
        // 绝不激活/抢焦点
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        // 避免背景擦除闪烁
        WM_ERASEBKGND => return 1,
        WM_DESTROY => {
            PostQuitMessage(0);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn create_host(main_hwnd: isize) -> Option<isize> {
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name = to_wide(HOST_CLASS);
        let wc = windows_sys::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: Some(host_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name.as_ptr(),
            style: 0,
            hCursor: std::ptr::null_mut(),
            hIcon: std::ptr::null_mut(),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            lpszMenuName: std::ptr::null(),
            cbClsExtra: 0,
            cbWndExtra: 0,
        };
        RegisterClassW(&wc); // 重复注册无害
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            main_hwnd as HWND, // owner：跟随主窗口最小化/销毁
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() { None } else { Some(hwnd as isize) }
    }
}

/// 启动舞台 UI 线程（创建窗口 + 消息循环），返回窗口句柄（mpv 的 wid）。
/// 必须在 mpv 初始化前调用；窗口在线程内创建，后续窗口操作也只在该线程执行。
///
/// 一次初始化只对应一代线程：失败回滚（`destroy()`）之后可以重来，重来前先把上一代
/// 等退干净 —— 两代同时活着就会有两扇覆盖窗口抢同一条命令队列，画面归谁说不清。
pub fn start_stage_thread(main_hwnd: isize) -> Option<isize> {
    wait_prev_stage_exit();
    *lock(&MAIN_HWND) = Some(main_hwnd);
    let (tx, rx) = std::sync::mpsc::channel::<isize>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    *lock(&EXIT_RX) = Some(done_rx);
    std::thread::Builder::new()
        .name("kx-stage".into())
        .spawn(move || {
            let hwnd = match create_host(main_hwnd) {
                Some(h) => h,
                None => {
                    let _ = tx.send(0);
                    let _ = done_tx.send(());
                    return;
                }
            };
            MY_HOST.with(|c| c.set(hwnd));
            *lock(&HOST) = Some(hwnd);
            let _ = tx.send(hwnd);
            message_loop();
            let _ = done_tx.send(());
        })
        .ok()?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(h) if h != 0 => Some(h),
        _ => None,
    }
}

/// 等上一代舞台线程退出。退出信号由线程自己在消息循环结束后发出（见 start_stage_thread）。
/// 等不到只记日志、不无限等：卡在这里等于把「初始化失败」升级成「应用起不来」，
/// 那比多留一个已作废的窗口更糟。
fn wait_prev_stage_exit() {
    let rx = lock(&EXIT_RX).take();
    if let Some(rx) = rx {
        if rx.recv_timeout(Duration::from_secs(3)).is_err() {
            crate::paths::append_log("[stage] 上一代舞台线程未在预期内退出，仍按新的一代继续");
        }
    }
}

fn message_loop() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r <= 0 {
                break; // WM_QUIT
            }
            if msg.message == WM_APP_STAGE {
                drain_and_apply();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn drain_and_apply() {
    let host = MY_HOST.with(std::cell::Cell::get);
    if host == 0 {
        return;
    }
    let main = lock(&MAIN_HWND).unwrap_or(0);
    loop {
        let cmd = match CMDS.lock() {
            Ok(mut q) => q.pop_front(),
            Err(_) => return,
        };
        match cmd {
            Some(StageCmd::Rect(r)) => apply_rect(main, host, &r),
            Some(StageCmd::Visible(v)) => set_os_visible(host, v),
            Some(StageCmd::Attach(target)) => apply_attach(host, target),
            Some(StageCmd::Destroy) => {
                unsafe { DestroyWindow(host as HWND) };
                // 挂靠目标复位：重来的一代从「挂主窗口」这个初始状态开始
                *lock(&TARGET) = None;
                return;
            }
            None => return,
        }
    }
}

/// 入队策略（纯函数，测试直接钉住）：
/// - `Rect` 是绝对几何：排队中的旧矩形在执行前就已经过时。拖一次窗口能发几十条，
///   全留着只会让画面追着历史位置跑（每条都要一次 SetWindowPos），故只留最后一条。
/// - `Visible`/`Attach`/`Destroy` 有先后语义，一条都不合并。
/// - 超过上限时丢**最旧**的一条并返回 true（丢历史、留最新意图，与 Rect 合并同一道理）。
fn push_cmd(q: &mut VecDeque<StageCmd>, cmd: StageCmd) -> bool {
    if matches!(cmd, StageCmd::Rect(_)) {
        q.retain(|c| !matches!(c, StageCmd::Rect(_)));
    }
    let mut dropped = false;
    while q.len() >= MAX_PENDING_CMDS {
        q.pop_front();
        dropped = true;
    }
    q.push_back(cmd);
    dropped
}

fn enqueue(cmd: StageCmd) {
    let dropped = push_cmd(&mut lock(&CMDS), cmd);
    if dropped {
        // 锁外记日志：append_log 要动文件，不在持锁时做
        crate::paths::append_log("[stage] 命令队列已满，丢弃最旧的一条");
    }
    if let Some(h) = *lock(&HOST) {
        unsafe { PostMessageW(h as HWND, WM_APP_STAGE, 0, 0) };
    }
}

fn apply_rect(main_hwnd: isize, host: isize, rect: &StageRect) {
    unsafe {
        if rect.visible && rect.w > 0 && rect.h > 0 {
            // 坐标相对「当前挂靠目标窗口」的客户区换算成屏幕坐标
            let base = lock(&TARGET).unwrap_or(main_hwnd);
            let mut pt = POINT { x: rect.x, y: rect.y };
            ClientToScreen(base as HWND, &mut pt);
            SetWindowPos(
                host as HWND,
                std::ptr::null_mut(),
                pt.x,
                pt.y,
                rect.w,
                rect.h,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            set_os_visible(host, true);
            // 垫底模式下每帧重断言一次层序：期间可能有别的应用把 topmost 窗口提到最前，
            // 漏掉这一步会让浮窗被自己的视频层盖住（表现为小窗黑屏/只有声音）。
            if let Some(pip) = *lock(&TARGET) {
                raise_pip(pip);
            }
        } else {
            ShowWindow(host as HWND, SW_HIDE);
        }
    }
}

/// 切换挂靠目标（在舞台线程执行）：
/// - 目标 = pip 悬浮窗：宿主窗口「垫」在浮窗之下，视频透过浮窗的透明像素显示，
///   浮窗内的 DOM 控件因此能浮在画面上。
/// - 目标 = None：挂回主窗口，恢复「覆盖」层序（压在 WebView 之上）。
///
/// owner 关系只在同一个 z 序带内生效，而顶层窗口先按「topmost 带 / 普通带」分层：
/// 悬浮窗是 always_on_top（topmost 带），宿主窗口若留在普通带就会被浮窗压在下面
/// （表现为小窗只有声音/黑屏）。所以挂靠浮窗时要把宿主窗口一并提进 topmost 带，
/// 挂回主窗口时再撤销，避免视频浮在其它应用之上。
///
/// 注意 owner 恒为主窗口：owned 窗口恒在其 owner **之上**，若以 pip 为 owner 就无法垫底。
fn apply_attach(host: isize, target: Option<isize>) {
    unsafe {
        let main = lock(&MAIN_HWND).unwrap_or(0);
        *lock(&TARGET) = target;
        if main != 0 {
            SetWindowLongPtrW(host as HWND, GWLP_HWNDPARENT, main);
        }
        SetWindowPos(
            host as HWND,
            if target.is_some() { HWND_TOPMOST } else { HWND_NOTOPMOST },
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOMOVE,
        );
        if let Some(pip) = target {
            raise_pip(pip);
        }
    }
}

/// 把 pip 窗口提到宿主窗口之上（垫底关系的另一半）。
/// 跨线程 SetWindowPos：目标线程（Tauri 主线程）始终在泵消息，且本函数只在
/// 挂靠/几何同步这些低频路径上调用，不会与主线程形成互相等待。
fn raise_pip(pip: isize) {
    unsafe {
        // 宿主刚被提到 topmost 带顶端，紧随其后把浮窗也提一次，浮窗即落在宿主之上
        SetWindowPos(
            pip as HWND,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOMOVE,
        );
    }
}

/// 设置舞台矩形（客户区物理像素 → 屏幕坐标）。异步：仅入队，由舞台线程执行。
pub fn set_stage_rect(rect: StageRect) {
    enqueue(StageCmd::Rect(rect));
}

/// 只切换可见性（主窗口失焦/聚焦时用）
pub fn set_os_visible_cmd(visible: bool) {
    enqueue(StageCmd::Visible(visible));
}

/// 切换覆盖窗口挂靠目标（None=主窗口）。异步：仅入队，由舞台线程执行。
pub fn attach(target: Option<isize>) {
    enqueue(StageCmd::Attach(target));
}

fn set_os_visible(host: isize, visible: bool) {
    unsafe {
        ShowWindow(host as HWND, if visible { 4 /* SW_SHOWNOACTIVATE */ } else { SW_HIDE });
    }
}

/// 销毁覆盖窗口。只入队：窗口由它自己的线程销毁（跨线程 DestroyWindow 会死等），
/// 那条命令跑完线程即退出。要重来的一代由 [`start_stage_thread`] 负责等它退干净。
pub fn destroy() {
    enqueue(StageCmd::Destroy);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32) -> StageCmd {
        StageCmd::Rect(StageRect { x, y: 0, w: 320, h: 180, visible: true })
    }

    /// 队列内容压成可比较的数字：矩形取 x，离散命令各给一个固定记号。
    fn digest(q: &VecDeque<StageCmd>) -> Vec<i32> {
        q.iter()
            .map(|c| match c {
                StageCmd::Rect(r) => r.x,
                StageCmd::Visible(_) => -1,
                StageCmd::Attach(_) => -2,
                StageCmd::Destroy => -3,
            })
            .collect()
    }

    /// 拖窗口时前端一帧一条矩形，早投的几条在执行前就已经过时：
    /// 全留着既拖慢追赶，又让画面先跳到历史位置再跳回来。
    #[test]
    fn queued_rects_coalesce_to_the_newest() {
        let mut q = VecDeque::new();
        for x in [10, 20, 30, 40] {
            push_cmd(&mut q, rect(x));
        }
        assert_eq!(digest(&q), vec![40], "只留最后一条矩形");
    }

    /// 合并只碰矩形：可见性/挂靠/销毁是离散动作，改顺序就会做出用户没要求的事。
    #[test]
    fn coalescing_a_rect_keeps_the_discrete_commands_around_it() {
        let mut q = VecDeque::new();
        push_cmd(&mut q, rect(10));
        push_cmd(&mut q, StageCmd::Visible(false));
        push_cmd(&mut q, rect(20));
        assert_eq!(digest(&q), vec![-1, 20], "老矩形被换掉，可见性命令原位保留");

        push_cmd(&mut q, StageCmd::Attach(Some(77)));
        push_cmd(&mut q, rect(30));
        assert_eq!(digest(&q), vec![-1, -2, 30], "挂靠命令一条都不许被吃掉");
    }

    #[test]
    fn the_queue_has_a_hard_cap_that_drops_the_oldest_first() {
        let mut q = VecDeque::new();
        for i in 0..MAX_PENDING_CMDS {
            let dropped = push_cmd(&mut q, StageCmd::Attach(Some(i as isize)));
            assert!(!dropped, "达到上限之前不该丢东西");
        }
        assert_eq!(q.len(), MAX_PENDING_CMDS);
        let dropped = push_cmd(&mut q, StageCmd::Destroy);
        assert!(dropped, "超限必须报告，交给调用方记日志");
        assert_eq!(q.len(), MAX_PENDING_CMDS, "上限是硬的");
        assert_eq!(digest(&q)[0], -2, "丢的是最旧的一条");
        assert_eq!(digest(&q).last().copied(), Some(-3), "最新意图一定在队尾");
    }
}
