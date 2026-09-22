//! 文件监听：notify 防抖 1s → 发 scanner:fsChanged。
//!
//! 只放行**能改变曲库**的路径（见 `matters`）：字幕/歌词落盘不属于曲库口径，
//! 却会成百次地打到监听上 —— AI 翻译每写一个 `.zh.lrc` 都在 1.2 s 后拉起一次全库
//! `startup_sync`，翻译期间界面被钉死（第 12 批修的就是这条）。

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Emitter;

/// 这个路径的变化可能改变曲库吗？
///
/// 后缀是音频/视频 → 相关；有别的后缀（`.lrc` / `.srt` / `.zh.vtt` / `.txt` / 封面图）
/// → 旁挂文件，曲库口径里根本没有它，忽略。**没后缀就当目录**放行：文件夹改名/删除是
/// 曲库级变更，宁可多扫一次。残留代价是「名字里带点的文件夹」被改名时会漏一次自动刷新
/// （界面「重新扫描」和 AI 页改名后的显式 `library.rescan(true)` 都还兜着）。
fn matters(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => crate::scanner::is_media_ext(ext),
        None => true,
    }
}

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
        let Ok(event) = res else { return };
        // 旁挂文件（含 AI 翻译写出的 .zh.*）不值得为它重扫全库
        if !event.paths.iter().any(|p| matters(p)) {
            return;
        }
        {
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

#[cfg(test)]
mod tests {
    use super::matters;
    use std::path::Path;

    /// 第 12 批冻结的根因：AI 翻译往曲库目录里批量写 `.zh.lrc`，每一条都在 1.2 s 后
    /// 拉起一次全库 `startup_sync`，界面被钉死到整条队列跑完。
    #[test]
    fn sidecar_files_do_not_trigger_a_rescan() {
        for p in [
            "C:/Music/Album/a.flac.zh.lrc",
            "C:/Music/Album/a.srt",
            "C:/Music/Album/a.zh.vtt",
            "C:/Music/Album/cover.jpg",
            "C:/Music/Album/_log.txt",
        ] {
            assert!(!matters(Path::new(p)), "{p} 不在曲库口径里");
        }
    }

    #[test]
    fn media_files_and_dirs_still_do() {
        assert!(matters(Path::new("C:/Music/Album/a.flac")));
        assert!(matters(Path::new("C:/Music/Album/b.mkv")));
        assert!(matters(Path::new("C:/Music/Artist")), "无后缀按目录处理，改名/删除不能漏");
        assert!(matters(Path::new("C:/Music")));
    }
}
