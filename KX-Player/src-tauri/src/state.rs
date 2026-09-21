//! 全局共享状态：应用句柄、强制退出标记、窗口最大化记忆、启动自检提示。

use crate::watcher::WatcherState;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct AppWindowsState {
    pub force_close: AtomicBool,
    pub last_maximized: AtomicBool,
    pub watcher: Arc<WatcherState>,
}

impl AppWindowsState {
    pub fn new() -> Self {
        Self {
            force_close: AtomicBool::new(false),
            last_maximized: AtomicBool::new(false),
            watcher: Arc::new(WatcherState::default()),
        }
    }
}

impl Default for AppWindowsState {
    fn default() -> Self {
        Self::new()
    }
}

/// 模块级 AppHandle（watcher 回调需要；Tauri 没有全局句柄时在 setup 里写入）
static APP: once_cell::sync::OnceCell<tauri::AppHandle> = once_cell::sync::OnceCell::new();

pub fn set_app_handle(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

pub fn app_handle() -> Option<&'static tauri::AppHandle> {
    APP.get()
}

/// P0-64 启动自检的降级提示。setup 阶段收集，前端首屏挂载后一次性取走
/// （此刻 WebView 还没起来，直接 emit 会被丢掉）。
static STARTUP_WARNINGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn startup_warn(msg: impl Into<String>) {
    let mut q = STARTUP_WARNINGS.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() < 20 {
        q.push(msg.into());
    }
}

pub fn take_startup_warnings() -> Vec<String> {
    let mut q = STARTUP_WARNINGS.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *q)
}
