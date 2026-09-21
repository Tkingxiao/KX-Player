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

use once_cell::sync::OnceCell;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
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

enum StageCmd {
    Rect(StageRect),
    Visible(bool),
    /// 切换覆盖窗口的挂靠目标：Some(hwnd)=独立悬浮窗（pip），None=主窗口。
    /// 同时把覆盖窗口的 owner 切到目标窗口，保证 z 序在目标之上、且随目标最小化而隐藏。
    Attach(Option<isize>),
    Destroy,
}

static CMDS: Mutex<VecDeque<StageCmd>> = Mutex::new(VecDeque::new());
static HOST: OnceCell<isize> = OnceCell::new();
static MAIN_HWND: OnceCell<isize> = OnceCell::new();
/// 当前挂靠目标窗口（None=主窗口）。坐标换算（ClientToScreen）以目标窗口的客户区为基准。
static TARGET: Mutex<Option<isize>> = Mutex::new(None);

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
pub fn start_stage_thread(main_hwnd: isize) -> Option<isize> {
    let _ = MAIN_HWND.set(main_hwnd);
    let (tx, rx) = std::sync::mpsc::channel::<isize>();
    std::thread::Builder::new()
        .name("kx-stage".into())
        .spawn(move || {
            let hwnd = match create_host(main_hwnd) {
                Some(h) => h,
                None => {
                    let _ = tx.send(0);
                    return;
                }
            };
            let _ = HOST.set(hwnd);
            let _ = tx.send(hwnd);
            message_loop(hwnd);
        })
        .ok()?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(h) if h != 0 => Some(h),
        _ => None,
    }
}

fn message_loop(_host: isize) {
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
    let main = match MAIN_HWND.get() {
        Some(m) => *m,
        None => return,
    };
    let host = match HOST.get() {
        Some(h) => *h,
        None => return,
    };
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
                return;
            }
            None => return,
        }
    }
}

fn enqueue(cmd: StageCmd) {
    let ok = match CMDS.lock() {
        Ok(mut q) => {
            q.push_back(cmd);
            true
        }
        Err(_) => false,
    };
    if ok {
        if let Some(h) = HOST.get() {
            unsafe { PostMessageW(*h as HWND, WM_APP_STAGE, 0, 0) };
        }
    }
}

fn apply_rect(main_hwnd: isize, host: isize, rect: &StageRect) {
    unsafe {
        if rect.visible && rect.w > 0 && rect.h > 0 {
            // 坐标相对「当前挂靠目标窗口」的客户区换算成屏幕坐标
            let base = match TARGET.lock() {
                Ok(t) => t.unwrap_or(main_hwnd),
                Err(_) => main_hwnd,
            };
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
            if let Some(pip) = TARGET.lock().ok().and_then(|t| *t) {
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
        let main = MAIN_HWND.get().copied().unwrap_or(0);
        let _ = TARGET.lock().map(|mut t| *t = target);
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

pub fn destroy() {
    enqueue(StageCmd::Destroy);
}
