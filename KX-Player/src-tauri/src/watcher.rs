//! 文件监听：单条 debounce worker → 发 `scanner:fsChanged`（带这一批的脏路径）。
//!
//! 只放行**能改变曲库**的路径（见 `matters`）：字幕/歌词落盘不属于曲库口径，
//! 却会成百次地打到监听上 —— AI 翻译每写一个 `.zh.lrc` 都在 1.2 s 后拉起一次全库
//! `startup_sync`，翻译期间界面被钉死（第 12 批修的就是这条）。
//!
//! 第 31 批（P1-5）把「每个事件一条 sleep 线程」换成一条常驻 worker 线程：
//! 脏路径进 `BTreeSet`（去重 + 稳定顺序），攒满 1 s 窗口或攒满 `MAX_BATCH` 就发一批；
//! `start` / `stop` 各自推进代际，`stop` 换代际后**在途的批一律作废**（攒着没发的也丢）。
//! 线程风暴、旧事件污染、watch 失败被吞，三条在这里一次收口。

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;

use crate::error::{AppErrorCode, IpcError, IpcResult};

/// 合并窗口：一条事件进来后最多再等这么久；持续不断的事件也按窗口发，不会被饿死。
const DEBOUNCE: Duration = Duration::from_secs(1);
/// 一批的条数上限（也是事件载荷的上限）：整目录拷贝不必先把几万条路径攒进内存。
const MAX_BATCH: usize = 1024;

/// 收批的去处。生产是把这一批脏路径发给前端；测试换成记账闭包，不碰 Tauri。
type Sink = Arc<dyn Fn(Vec<String>) + Send + Sync>;

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
    /// start/stop 代际：两边都自增；worker 只在「代际还是自己那一代」时才发事件。
    gen: Arc<AtomicU64>,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self { watcher: Mutex::new(None), gen: Arc::new(AtomicU64::new(0)) }
    }
}

/// 挂监听：先推进代际、丢掉旧 watcher（旧 worker 随之退休），再逐根挂。
///
/// 返回**没挂上的根**（离线盘、目录不存在、权限不足）—— 命令层据此如实回话，不再
/// 无条件报成功；`Err` 只留给「监听器根本建不起来」这一种情形。
pub fn start(paths: &[String], state: &Arc<WatcherState>) -> IpcResult<Vec<String>> {
    start_with(paths, state, Arc::new(emit_fs_changed))
}

fn start_with(paths: &[String], state: &Arc<WatcherState>, sink: Sink) -> IpcResult<Vec<String>> {
    let my_gen = state.gen.fetch_add(1, Ordering::SeqCst) + 1;
    *state.watcher.lock() = None;
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let (tx, rx) = mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: Result<NotifyEvent, notify::Error>| {
        let Ok(event) = res else { return };
        // 旁挂文件（含 AI 翻译写出的 .zh.*）不值得为它重扫全库
        for p in event.paths.iter().filter(|p| matters(p)) {
            // worker 已退休（stop / 重启）时这里会失败：丢一条事件不是事故
            if tx.send(p.clone()).is_err() {
                return;
            }
        }
    })
    .map_err(|e| IpcError::with_detail(AppErrorCode::IoFailed, "文件监听无法启动", e))?;

    let mut failed = Vec::new();
    for p in paths {
        if let Err(e) = watcher.watch(Path::new(p), RecursiveMode::Recursive) {
            crate::paths::append_log(&format!("[watcher] 挂不上 {p}：{e}"));
            failed.push(p.clone());
        }
    }
    if failed.len() == paths.len() {
        crate::paths::append_log("[watcher] 没有一个根挂得上监听，本次不启动");
        return Ok(failed);
    }

    let gen = Arc::clone(&state.gen);
    std::thread::Builder::new()
        .name("fs-debounce".into())
        .spawn(move || worker(rx, gen, my_gen, DEBOUNCE, sink))
        .map_err(|e| IpcError::with_detail(AppErrorCode::Internal, "文件监听线程启动失败", e))?;
    *state.watcher.lock() = Some(watcher);
    Ok(failed)
}

/// 停监听：代际自增 → 在途的批全部作废（`stop` 之后攒着的那一批也不发）。
///
/// 「攒着的不发」是有意的：所有调用方（换根、移除目录、退出）在 stop 之后要么显式
/// 重扫一遍，要么本来就不再需要刷新 —— 旧事件不该在新一代里冒出来。
pub fn stop(state: &Arc<WatcherState>) {
    state.gen.fetch_add(1, Ordering::SeqCst);
    *state.watcher.lock() = None;
}

fn emit_fs_changed(paths: Vec<String>) {
    let Some(app) = crate::state::app_handle() else {
        crate::paths::append_log("[watcher] 有变化但没有 AppHandle，事件发不出去");
        return;
    };
    if let Err(e) = app.emit("scanner:fsChanged", paths) {
        crate::paths::append_log(&format!("[watcher] scanner:fsChanged 发送失败：{e}"));
    }
}

/// 单条 debounce worker：闲时阻塞等第一条，拿到就把窗口内（或满批前）的路径一起发出去。
fn worker(rx: Receiver<PathBuf>, gen: Arc<AtomicU64>, my_gen: u64, debounce: Duration, sink: Sink) {
    let mut dirty: BTreeSet<PathBuf> = BTreeSet::new();
    loop {
        if dirty.is_empty() {
            match rx.recv() {
                Ok(p) => {
                    dirty.insert(p);
                }
                Err(_) => return, // watcher 被丢（stop / 重启）：worker 退休
            }
            continue;
        }
        // 攒批：收后来者，但最多等一个窗口 —— 持续写入的目录不会把事件饿死
        let deadline = Instant::now() + debounce;
        loop {
            if dirty.len() >= MAX_BATCH {
                break; // 满批：不再等窗口，先把这一批发出去（内存有界）
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(p) => {
                    dirty.insert(p);
                }
                Err(RecvTimeoutError::Timeout) => break,
                // 断开 = 这一代监听已经收场：攒着的不发，直接退休
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        // 代际对不上 = 已被 stop/start 取代（发出前最后一道闸）
        if gen.load(Ordering::SeqCst) != my_gen {
            return;
        }
        let batch: Vec<String> = dirty.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        dirty.clear();
        sink(batch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn recorder() -> (Sink, Arc<StdMutex<Vec<Vec<String>>>>) {
        let batches: Arc<StdMutex<Vec<Vec<String>>>> = Arc::new(StdMutex::new(Vec::new()));
        let b = Arc::clone(&batches);
        let sink: Sink = Arc::new(move |paths: Vec<String>| b.lock().unwrap().push(paths));
        (sink, batches)
    }

    /// 轮询等 `f` 成真（最多 `ms` 毫秒）。文件系统事件没有「立刻」这一说，只能等。
    fn wait_until(ms: u64, mut f: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if f() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

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

    /// 一次窗口内的事件合成一批，重复路径只留一条（顺序稳定）。
    #[test]
    fn a_burst_becomes_one_batch_without_duplicates() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        std::thread::spawn(move || worker(rx, gen, 1, Duration::from_millis(60), sink));
        for p in ["C:/m/b.flac", "C:/m/a.flac", "C:/m/a.flac"] {
            tx.send(PathBuf::from(p)).expect("投事件");
        }
        assert!(wait_until(2000, || !batches.lock().unwrap().is_empty()), "窗口到点要发批");
        std::thread::sleep(Duration::from_millis(150)); // 再等一个窗口：不该有第二批
        let b = batches.lock().unwrap();
        assert_eq!(b.len(), 1, "同一窗口只发一批");
        assert_eq!(b[0], vec!["C:/m/a.flac".to_string(), "C:/m/b.flac".to_string()]);
    }

    /// 持续写入（拷贝/转录一整个目录）不能把事件饿到静默为止：按窗口一批批发，一条不丢。
    #[test]
    fn a_steady_stream_is_flushed_per_window() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        std::thread::spawn(move || worker(rx, gen, 1, Duration::from_millis(60), sink));
        let sent: Vec<String> = (0..12).map(|i| format!("C:/m/{i}.flac")).collect();
        for p in &sent {
            tx.send(PathBuf::from(p)).expect("投事件");
            std::thread::sleep(Duration::from_millis(20));
        }
        let want = sent.len();
        assert!(
            wait_until(2000, || batches.lock().unwrap().iter().flatten().count() >= want),
            "窗口内的批要发出来，不能一直等静默"
        );
        let b = batches.lock().unwrap();
        assert!(b.len() >= 2, "12 条 ×20ms 跨过多个 60ms 窗口，实际只有 {} 批", b.len());
        let mut got: Vec<String> = b.iter().flatten().cloned().collect();
        got.sort();
        let mut expect = sent.clone();
        expect.sort();
        assert_eq!(got, expect, "一条都不许丢");
    }

    /// 满批先发：不必等窗口到点（也是内存上界）。
    #[test]
    fn a_full_batch_is_flushed_without_waiting_out_the_window() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        // 窗口给 60 s：这批能在窗口前出来，只可能是「满批先发」
        std::thread::spawn(move || worker(rx, gen, 1, Duration::from_secs(60), sink));
        for i in 0..MAX_BATCH {
            tx.send(PathBuf::from(format!("C:/m/{i}.flac"))).expect("投事件");
        }
        assert!(wait_until(3000, || !batches.lock().unwrap().is_empty()), "满批不该等窗口");
        assert_eq!(batches.lock().unwrap()[0].len(), MAX_BATCH);
    }

    /// stop 之后 watcher 被丢：worker 退休，攒着的那一批不许再发。
    #[test]
    fn a_pending_batch_is_dropped_when_the_watcher_goes_away() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        let h = std::thread::spawn(move || worker(rx, gen, 1, Duration::from_millis(80), sink));
        tx.send(PathBuf::from("C:/m/a.flac")).expect("投事件");
        drop(tx); // 等价于 stop：watcher 被丢掉、回调里的 sender 一起没
        assert!(wait_until(2000, || h.is_finished()), "watcher 走后 worker 要退休");
        assert!(batches.lock().unwrap().is_empty(), "已收场的那一批不许再发");
    }

    /// 代际对不上 = 已被 stop/start 取代：连着已经攒好的那批一起作废。
    #[test]
    fn a_stale_generation_never_emits() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        let worker_gen = Arc::clone(&gen);
        let h = std::thread::spawn(move || worker(rx, worker_gen, 1, Duration::from_millis(80), sink));
        tx.send(PathBuf::from("C:/m/a.flac")).expect("投事件");
        gen.fetch_add(1, Ordering::SeqCst); // stop/start 换代际
        assert!(wait_until(2000, || h.is_finished()), "过代 worker 要退休");
        assert!(batches.lock().unwrap().is_empty(), "旧代的批不许打到前端");
    }

    /// 闲时阻塞等事件（不占 CPU），watcher 一走就干净退出。
    #[test]
    fn an_idle_worker_retires_when_its_watcher_goes_away() {
        let (sink, batches) = recorder();
        let (tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(1));
        let h = std::thread::spawn(move || worker(rx, gen, 1, Duration::from_millis(50), sink));
        drop(tx);
        assert!(wait_until(2000, || h.is_finished()));
        assert!(batches.lock().unwrap().is_empty());
    }

    /// P1-5 的「错误返回」：挂不上的根如实报回，一个都没挂上时不留半个监听器。
    #[test]
    fn start_reports_the_roots_it_could_not_watch() {
        let state = Arc::new(WatcherState::default());
        assert!(start(&[], &state).expect("空清单不算失败").is_empty(), "没有根要挂：不算失败");

        let missing = std::env::temp_dir()
            .join(format!("kx-watcher-missing-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let failed = start(std::slice::from_ref(&missing), &state).expect("建监听器本身不该失败");
        assert_eq!(failed, vec![missing], "挂不上的根要报回来，命令层才有话可说");
        assert!(state.watcher.lock().is_none(), "一个根都没挂上就不该留监听器");
    }

    /// 真 watcher + 真文件系统走一遍：事件 → matters 过滤 → 批量 sink。
    #[test]
    fn a_real_file_change_reaches_the_sink() {
        let dir = std::env::temp_dir().join(format!("kx-watcher-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let (sink, batches) = recorder();
        let state = Arc::new(WatcherState::default());
        let root = dir.to_string_lossy().into_owned();
        assert!(start_with(&[root], &state, sink).expect("挂临时目录").is_empty());

        let mut n = 0;
        let ok = wait_until(8000, || {
            if batches.lock().unwrap().iter().flatten().any(|p| p.ends_with(".flac")) {
                return true;
            }
            // 首写有可能落在监听注册完成之前，轮询时补写一个新文件
            n += 1;
            let _ = std::fs::write(dir.join(format!("t{n}.flac")), b"x");
            false
        });
        stop(&state);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "真实文件变化应当以脏路径的形式到 sink");
    }
}
