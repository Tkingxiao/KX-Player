//! 文件监听：notify 防抖 1s → 发 scanner:fsChanged。

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Emitter;

pub struct WatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
    debounce_gen: Arc<AtomicU64>,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self { watcher: Mutex::new(None), debounce_gen: Arc::new(AtomicU64::new(0)) }
    }
}

pub fn start(paths: &[String], state: Arc<WatcherState>) {
    stop(&state);
    let gen = state.debounce_gen.clone();
    let app = crate::state::app_handle().cloned();
    let mut watcher = match notify::recommended_watcher(move |res: Result<NotifyEvent, notify::Error>| {
        if res.is_ok() {
            // 抖动合并：1s 内多次事件只发一次
            let my = gen.fetch_add(1, Ordering::Relaxed);
            let gen2 = gen.clone();
            let app2 = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                // 1s 内又有新事件 → 放弃本次（由最新事件负责触发）
                if gen2.load(Ordering::Relaxed) == my + 1 {
                    if let Some(app) = app2 {
                        let _ = app.emit("scanner:fsChanged", ());
                    }
                }
            });
        }
    }) {
        Ok(w) => w,
        Err(_) => return,
    };
    for p in paths {
        let _ = watcher.watch(std::path::Path::new(p), RecursiveMode::Recursive);
    }
    *state.watcher.lock() = Some(watcher);
}

pub fn stop(state: &Arc<WatcherState>) {
    *state.watcher.lock() = None;
}
