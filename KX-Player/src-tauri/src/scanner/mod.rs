//! 扫描编排：文件发现 → 并行元数据解析 → 结构分组 → 封面落盘 → 快照入库。
//! 与 Electron fileScanner.ts 的算法逐段对齐（含三层封面、祖先链文件夹树、增量判据）。

pub mod covers;
pub mod meta;

use crate::db;
use crate::model::{normalize_path, Album, Artist, FolderNode, ScanResult, Track};
use crate::scanner::meta::*;
use crate::scanner::meta::ScanMeta;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const MAX_DIR_COUNT: usize = 2000;

/// >240 字符路径加 \\?\ 前缀（Windows MAX_PATH 旁路）
fn long_path(p: &str) -> String {
    if p.len() > 240 && !p.starts_with("\\\\?\\") {
        format!("\\\\?\\{}", p.replace('/', "\\"))
    } else {
        p.to_string()
    }
}

fn is_media_file(name: &str) -> bool {
    let ext = ext_of(name);
    AUDIO_EXTS.contains(&ext.as_str()) || VIDEO_EXTS.contains(&ext.as_str())
}

fn discover_files(folder_paths: &[String]) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = folder_paths.iter().map(|p| normalize_path(p)).collect();
    let start = std::time::Instant::now();
    while let Some(dir) = stack.pop() {
        if visited.contains(&dir) || visited.len() >= MAX_DIR_COUNT {
            continue;
        }
        // 防止极深目录/软链接循环卡住：单目录遍历限时 30 秒
        if start.elapsed().as_secs() > 30 {
            crate::paths::append_log("[scan] discover_files timed out (30s), returning partial results");
            break;
        }
        visited.insert(dir.clone());
        let entries = match std::fs::read_dir(Path::new(&long_path(&dir))) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let full = format!("{dir}/{name}");
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(full);
            } else if is_media_file(&name) {
                results.push(full);
                if results.len() >= 100_000 {
                    crate::paths::append_log("[scan] hit max 100000 media files limit");
                    results.sort();
                    return results;
                }
            }
        }
    }
    results.sort();
    results
}

#[derive(Debug, Clone, Default)]
pub struct FileMeta {
    pub duration: f64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
}

fn emit_progress(app: &AppHandle, completed: i64, total: i64) {
    let _ = app.emit("scanner:progress", serde_json::json!({ "completed": completed, "total": total }));
}

fn emit_stage(app: &AppHandle, stage: &str) {
    let _ = app.emit("scanner:stage", stage.to_string());
}

/// 单文件解析硬超时（与旧实现一致：10 秒），由主线程巡检判定并兜底。
const PARSE_TIMEOUT: Duration = Duration::from_secs(10);
/// 主线程巡检间隔：50ms 足以在超时后第一时间接管，开销可忽略。
const WATCHDOG_TICK: Duration = Duration::from_millis(50);
/// 进度事件节流间隔（保持原实现的 120ms）。
const PROGRESS_TICK: Duration = Duration::from_millis(120);

/// worker 的当前工作登记：(文件索引, 所属 chunk 结束索引, 开始时刻)。
/// 主线程巡检读取它，判断某个 worker 是否卡在同一文件上超过 `PARSE_TIMEOUT`。
type WorkerSlot = Arc<Mutex<Option<(usize, usize, Instant)>>>;

/// 活跃 worker 计数守卫：无论正常退出还是 panic 展开都会递减，
/// 主线程因此不会在 worker 全部消失后继续空等。
struct ActiveWorkers(Arc<AtomicUsize>);

impl Drop for ActiveWorkers {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn meta_to_file_meta(m: ScanMeta) -> FileMeta {
    FileMeta {
        duration: m.duration,
        title: m.title,
        artist: m.artist,
        genre: m.genre,
        bitrate: m.bitrate,
        sample_rate: m.sample_rate,
    }
}

/// 解析单个文件并把结果计入共享状态。
/// - `parse` 的 panic 被捕获后降级为兜底元数据，扫描不会因此中断；
/// - 结果认领用 `finished[idx]` 的 CAS 完成：worker 正常完成与 watchdog 超时兜底
///   只有一方能计数；已判超时的文件不会被 worker 迟到的真实结果覆盖（与旧版语义一致）。
///
/// 参数表就是「共享状态 + 本次认领范围」的完整清单，打包成结构体只是把同一批字段换个地方写。
#[allow(clippy::too_many_arguments)]
fn parse_one_guarded(
    idx: usize,
    chunk_end: usize,
    files: &[String],
    parse: &(dyn Fn(&str) -> ScanMeta + Send + Sync),
    slot: &Mutex<Option<(usize, usize, Instant)>>,
    finished: &[AtomicBool],
    results: &Mutex<HashMap<String, FileMeta>>,
    counter: &AtomicUsize,
) {
    let path = &files[idx];
    // 已被 watchdog 兜底或其它 worker 认领（重复退回项）→ 不再解析
    if finished[idx].load(Ordering::Acquire) {
        return;
    }
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((idx, chunk_end, Instant::now()));
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(path)));
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;

    let meta = match parsed {
        Ok(m) => meta_to_file_meta(m),
        Err(_) => {
            crate::paths::append_log(&format!("[scan] parse failed: {path}"));
            meta_to_file_meta(timeout_meta(path))
        }
    };
    if finished[idx]
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        results.lock().unwrap_or_else(|e| e.into_inner()).insert(path.clone(), meta);
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// 并行解析池：固定 worker 池 + 共享 chunk 游标 + 主线程超时巡检。
///
/// 与旧实现的关键区别：不再为**每个文件** spawn 一个线程（旧版每文件一线程 +
/// `recv_timeout`，大库下线程创建/销毁是纯开销），改为 worker 只创建一次、
/// 各自从共享游标领取 chunk；单文件超时改由主线程巡检（watchdog）负责：
/// 某个 worker 在同一个文件上停留超过 `timeout`，即记日志、写兜底元数据、计入进度，
/// 并把该 chunk 剩下的文件退回队列 + 补一个 worker 维持并发度。
/// 只有真正卡死的文件才会触发补位，因此不会退化成「每文件一线程」。
///
/// `parse` 与 `report` 以 trait object 注入，便于单测；生产入口见 `parse_files_parallel`。
fn parse_pool(
    files: &[String],
    timeout: Duration,
    report: Arc<dyn Fn(usize) + Send + Sync>,
    parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync>,
) -> HashMap<String, FileMeta> {
    let total = files.len();
    if total == 0 {
        report(0);
        return HashMap::new();
    }

    // worker 数：min(cpu, 8)，下限 2（宪法：扫描解码并发 min(cpu, 8)）。
    // 上限 8 避免解码/IO 争抢与内存峰值；下限 2 让单核机器也能让 IO 与 CPU 重叠。
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    // 领取粒度：把任务切成约 4 个 chunk/worker —— chunk 越小负载越均衡（个别慢文件不至于
    // 形成长尾），但每个 chunk 至少 1 个文件。worker 数再按 chunk 数封顶，
    // 于是小库（total < threads*4）不会 spawn 一堆空转线程，也不会出现空 chunk。
    let chunk_size = std::cmp::max(1, total.div_ceil(threads * 4));
    let chunk_count = total.div_ceil(chunk_size);
    let worker_count = std::cmp::min(threads, chunk_count);

    let files: Arc<Vec<String>> = Arc::new(files.to_vec());
    let results: Arc<Mutex<HashMap<String, FileMeta>>> = Arc::new(Mutex::new(HashMap::with_capacity(total)));
    let counter = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    // 每个文件的「已计入结果」标志：worker 正常完成与 watchdog 超时兜底用它 CAS 竞争唯一认领权
    let finished: Arc<Vec<AtomicBool>> = Arc::new((0..total).map(|_| AtomicBool::new(false)).collect());
    // 活跃 worker 的工作登记（补位线程各自一个新槽位，避免旧槽位归还时覆盖新登记）
    let slots: Arc<Mutex<Vec<WorkerSlot>>> = Arc::new(Mutex::new(Vec::new()));
    // 共享工作游标（chunk 粒度）
    let next_chunk = Arc::new(AtomicUsize::new(0));
    // watchdog 退回的单文件索引（卡死 worker 未处理完的 chunk 尾巴），worker 优先领取
    let requeued: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

    // 每次调用都创建独立槽位并启动一个 worker；同时被复用为超时后的补位。
    // 用 detach 线程（非 scope）：被判定卡死的 worker 不能阻塞整体返回。
    let spawn_worker = {
        let files = Arc::clone(&files);
        let results = Arc::clone(&results);
        let counter = Arc::clone(&counter);
        let active = Arc::clone(&active);
        let finished = Arc::clone(&finished);
        let slots = Arc::clone(&slots);
        let next_chunk = Arc::clone(&next_chunk);
        let requeued = Arc::clone(&requeued);
        let parse = Arc::clone(&parse);
        move || {
            let slot: WorkerSlot = Arc::new(Mutex::new(None));
            slots.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::clone(&slot));
            active.fetch_add(1, Ordering::Relaxed);
            let files = Arc::clone(&files);
            let results = Arc::clone(&results);
            let counter = Arc::clone(&counter);
            let finished = Arc::clone(&finished);
            let next_chunk = Arc::clone(&next_chunk);
            let requeued = Arc::clone(&requeued);
            let parse = Arc::clone(&parse);
            let worker_active = Arc::clone(&active);
            let spawned = std::thread::Builder::new().name("scan-parse".into()).spawn(move || {
                let _guard = ActiveWorkers(worker_active);
                loop {
                    // 优先处理 watchdog 退回的单文件尾巴
                    let single = requeued.lock().unwrap_or_else(|e| e.into_inner()).pop();
                    match single {
                        Some(idx) => parse_one_guarded(
                            idx,
                            idx + 1,
                            &files,
                            parse.as_ref(),
                            &slot,
                            finished.as_slice(),
                            &results,
                            &counter,
                        ),
                        None => {
                            let c = next_chunk.fetch_add(1, Ordering::Relaxed);
                            if c >= chunk_count {
                                break;
                            }
                            let start = c * chunk_size;
                            let end = std::cmp::min(start + chunk_size, total);
                            for idx in start..end {
                                parse_one_guarded(
                                    idx,
                                    end,
                                    &files,
                                    parse.as_ref(),
                                    &slot,
                                    finished.as_slice(),
                                    &results,
                                    &counter,
                                );
                            }
                        }
                    }
                }
            });
            if spawned.is_err() {
                crate::paths::append_log("[scan] failed to spawn parse worker");
                active.fetch_sub(1, Ordering::Relaxed);
            }
        }
    };

    for _ in 0..worker_count {
        spawn_worker();
    }

    report(0);
    let mut last_emit = Instant::now();
    // 退回队列的补位只做一次：极端情况下（线程创建持续失败）保证循环必然终止
    let mut respawned_for_requeue = false;

    // 主线程即 watchdog：巡检 worker 登记 + 按 120ms 节流上报进度
    loop {
        let observed: Vec<WorkerSlot> = slots.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for slot in observed {
            // 原子地「取走」一个仍处于超时状态的登记：取走后该 worker 之后的新登记
            // （下一个文件）不会被这里误清；worker 自己收尾时再写 None 也是幂等的。
            let stuck = {
                let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
                match *guard {
                    Some((idx, end, started)) if started.elapsed() >= timeout => {
                        *guard = None;
                        Some((idx, end))
                    }
                    _ => None,
                }
            };
            let Some((idx, end)) = stuck else { continue };
            if finished[idx]
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                crate::paths::append_log(&format!("[scan] parse timeout: {}", files[idx]));
                results.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    files[idx].clone(),
                    meta_to_file_meta(timeout_meta(&files[idx])),
                );
                counter.fetch_add(1, Ordering::Relaxed);
                // 被放弃的 worker 仍卡在 parse 里回不来：把该 chunk 剩余文件退回队列，
                // 并补一个 worker，保证这个 chunk 尾巴上的文件不被一个卡死文件拖住。
                if end > idx + 1 {
                    let mut q = requeued.lock().unwrap_or_else(|e| e.into_inner());
                    for j in (idx + 1)..end {
                        q.push(j);
                    }
                }
                spawn_worker();
            }
        }

        let done = counter.load(Ordering::Relaxed);
        if last_emit.elapsed() >= PROGRESS_TICK {
            report(done);
            last_emit = Instant::now();
        }

        if done >= total {
            break;
        }
        // 没有存活 worker 且仍有文件未计数 → 退出，不无限等待。
        // 若退回队列还有尾巴（补位线程创建失败等极端情况），先补一次 worker 尝试排空；
        // 补位也失败则直接退出，保证必然会返回（不会挂死）。
        if active.load(Ordering::Relaxed) == 0 {
            let pending = requeued.lock().unwrap_or_else(|e| e.into_inner()).len();
            if pending > 0 && !respawned_for_requeue {
                respawned_for_requeue = true;
                for _ in 0..std::cmp::min(worker_count, pending) {
                    spawn_worker();
                }
                continue;
            }
            crate::paths::append_log(&format!("[scan] parse workers exited early: {done}/{total} parsed"));
            break;
        }
        std::thread::sleep(WATCHDOG_TICK);
    }

    let done = counter.load(Ordering::Relaxed);
    report(done);
    crate::paths::append_log(&format!(
        "[scan] parsed {done}/{total} files ({worker_count} workers, chunk={chunk_size}, cpu={threads})"
    ));
    let out = results.lock().unwrap_or_else(|e| e.into_inner()).clone();
    out
}

/// 扫描用并行解析入口：worker 池 + 逐文件 10s 硬超时，进度每 120ms 节流上报。
fn parse_files_parallel(files: &[String], app: &AppHandle, total_hint: i64, done_offset: i64) -> HashMap<String, FileMeta> {
    let app = app.clone();
    let report: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |done: usize| {
        emit_progress(&app, done_offset + done as i64, total_hint);
    });
    parse_pool(files, PARSE_TIMEOUT, report, Arc::new(parse_file))
}

/// 超时兜底元数据（与旧版 parse_one 的超时分支完全一致）
fn timeout_meta(path: &str) -> ScanMeta {
    ScanMeta {
        duration: 0.0,
        cover: None,
        title: Some(crate::scanner::meta::basename_no_ext(path)),
        artist: None,
        genre: None,
        bitrate: None,
        sample_rate: None,
    }
}

struct ScannedAlbum {
    name: String,
    artist: String,
    dir_path: Option<String>,
    cover_file: Option<String>,
    tracks: Vec<Track>,
}

struct ScannedArtist {
    name: String,
    path: String,
    albums: Vec<ScannedAlbum>,
}

fn album_dir_for(root: &str, parts: &[&str]) -> String {
    if parts.len() >= 2 {
        let mut p = root.to_string();
        for part in &parts[..parts.len() - 1] {
            p = format!("{p}/{part}");
        }
        p
    } else {
        root.to_string()
    }
}

/// 艺术家/专辑分组（专辑 = 音轨直接父目录；标签 artist 覆盖目录名）
fn group_tracks(files: &[String], metas: &HashMap<String, FileMeta>, roots: &[String]) -> Vec<ScannedArtist> {
    let normalized_roots: Vec<String> = roots.iter().map(|r| normalize_path(r)).collect();
    let mut artist_map: HashMap<String, (String, HashMap<String, ScannedAlbum>)> = HashMap::new();

    for fp in files {
        let Some(m) = metas.get(fp) else { continue };
        let nfp = normalize_path(fp);
        let Some(root) = normalized_roots.iter().find(|r| nfp == **r || nfp.starts_with(&format!("{r}/"))) else {
            continue;
        };
        let rel = &nfp[root.len()..].trim_start_matches('/');
        let parts: Vec<&str> = if rel.is_empty() { vec![] } else { rel.split('/').collect() };
        let mut artist_name = root.rsplit('/').next().unwrap_or(root).to_string();
        let album_name = if parts.len() >= 2 { parts[parts.len() - 2].to_string() } else { artist_name.clone() };
        if let Some(a) = m.artist.as_ref().filter(|s| !s.trim().is_empty()) {
            artist_name = a.trim().to_string();
        }

        let entry = artist_map.entry(artist_name.clone()).or_insert_with(|| (root.clone(), HashMap::new()));
        let albums = &mut entry.1;
        let album = albums.entry(album_name.clone()).or_insert_with(|| ScannedAlbum {
            name: album_name.clone(),
            artist: artist_name.clone(),
            dir_path: Some(album_dir_for(root, &parts)),
            cover_file: None,
            tracks: Vec::new(),
        });
        let ext = ext_of(fp);
        album.tracks.push(Track {
            id: file_id(fp),
            name: m.title.as_deref().filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).unwrap_or_else(|| normalize_display_name(fp)),
            path: nfp,
            duration: m.duration,
            artist: m.artist.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "佚名".into()),
            album: album_name,
            format: ext,
            is_video: is_video(fp),
            cover_path: None,
            cover_data: None,
            lyrics_path: None,
            file_mtime: stat_file(fp).map(|s| s.0).unwrap_or(0.0),
            file_size: stat_file(fp).map(|s| s.1).unwrap_or(0),
            meta_title: m.title.clone(),
            meta_artist: m.artist.clone(),
            genre: m.genre.clone(),
            bitrate: m.bitrate,
            sample_rate: m.sample_rate,
            loudness_lufs: None,
            album_cover_data: None,
        });
    }

    artist_map
        .into_iter()
        .map(|(name, (path, albums))| ScannedArtist {
            name,
            path,
            albums: albums.into_values().collect(),
        })
        .collect()
}

fn build_folder_tree(files: &[String], metas: &HashMap<String, FileMeta>, roots: &[String]) -> Vec<FolderNode> {
    let clean_roots: Vec<String> = roots.iter().map(|r| normalize_path(r)).collect();
    let mut node_map: HashMap<String, FolderNode> = HashMap::new();

    fn get_or_create(map: &mut HashMap<String, FolderNode>, path: &str) {
        if !map.contains_key(path) {
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            map.insert(path.to_string(), FolderNode {
                name,
                path: path.to_string(),
                children: Vec::new(),
                tracks: Vec::new(),
                track_count: 0,
                cover_data: None,
            });
        }
    }

    for fp in files {
        let Some(m) = metas.get(fp) else { continue };
        let nfp = normalize_path(fp);
        let Some(_root) = clean_roots.iter().find(|r| nfp == **r || nfp.starts_with(&format!("{r}/"))) else {
            continue;
        };
        let dir = match nfp.rfind('/') {
            Some(i) => nfp[..i].to_string(),
            None => nfp.clone(),
        };
        get_or_create(&mut node_map, &dir);
        let ext = ext_of(fp);
        let dir_name = dir.rsplit('/').next().unwrap_or(&dir).to_string();
        if let Some(node) = node_map.get_mut(&dir) {
            node.tracks.push(Track {
                id: file_id(fp),
                name: m.title.as_deref().filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).unwrap_or_else(|| normalize_display_name(fp)),
                path: nfp,
                duration: m.duration,
                artist: m.artist.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "佚名".into()),
                album: dir_name,
                format: ext,
                is_video: is_video(fp),
                cover_path: None,
                cover_data: None,
                lyrics_path: None,
                file_mtime: stat_file(fp).map(|s| s.0).unwrap_or(0.0),
                file_size: stat_file(fp).map(|s| s.1).unwrap_or(0),
                meta_title: m.title.clone(),
                meta_artist: m.artist.clone(),
                genre: m.genre.clone(),
                bitrate: m.bitrate,
                sample_rate: m.sample_rate,
                loudness_lufs: None,
                album_cover_data: None,
            });
        }
        // 祖先链：确保从叶子目录到根的所有中间目录节点存在
        // （children 挂接统一由后方的排序链接循环完成，父目录路径必然先于子目录处理）
        let mut cur = dir.clone();
        while let Some(parent_end) = cur.rfind('/') {
            let parent = cur[..parent_end].to_string();
            if parent.is_empty() {
                break;
            }
            get_or_create(&mut node_map, &cur);
            get_or_create(&mut node_map, &parent);
            if clean_roots.contains(&parent) {
                break;
            }
            cur = parent;
        }
    }

    // 统一挂接：先建 parent→children 关系表，再从根递归组装
    // （不能边遍历边 remove——父节点被移出 map 后其子节点会失联成孤儿根）
    let mut map = node_map;
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut root_paths: Vec<String> = Vec::new();
    for key in map.keys() {
        let parent = key.rfind('/').map(|i| key[..i].to_string());
        match parent {
            Some(p) if map.contains_key(&p) => children_map.entry(p).or_default().push(key.clone()),
            _ => root_paths.push(key.clone()),
        }
    }

    fn assemble(
        path: &str,
        nodes: &mut HashMap<String, FolderNode>,
        children_map: &HashMap<String, Vec<String>>,
    ) -> FolderNode {
        let mut node = nodes.remove(path).unwrap_or_else(|| FolderNode {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            children: Vec::new(),
            tracks: Vec::new(),
            track_count: 0,
            cover_data: None,
        });
        if let Some(kids) = children_map.get(path) {
            for kid in kids {
                node.children.push(assemble(kid, nodes, children_map));
            }
        }
        node.children.sort_by_key(|a| a.name.to_lowercase());
        node.tracks.sort_by_key(|a| a.name.to_lowercase());
        node
    }

    let mut roots_sorted: Vec<FolderNode> = Vec::new();
    for rp in &root_paths {
        roots_sorted.push(assemble(rp, &mut map, &children_map));
    }

    // 排序 + 计数
    fn finalize(node: &mut FolderNode) -> i64 {
        node.children.sort_by_key(|a| a.name.to_lowercase());
        node.tracks.sort_by_key(|a| a.name.to_lowercase());
        let mut count = node.tracks.len() as i64;
        for child in &mut node.children {
            count += finalize(child);
        }
        node.track_count = count;
        count
    }
    for r in &mut roots_sorted {
        finalize(r);
    }
    roots_sorted.sort_by_key(|a| a.name.to_lowercase());
    // 仅保留有内容的根
    roots_sorted.retain(|r| r.track_count > 0 || !r.children.is_empty());
    roots_sorted
}

/// 三级封面：缓存 → 外部文件（向上回溯 3 级）→ 内嵌重解析
fn fill_album_covers(artists: &mut [ScannedArtist], only_changed: Option<&HashSet<String>>, load_cached: bool) {
    let existing: HashSet<String> = if load_cached { covers::existing_track_cover_ids() } else { HashSet::new() };
    for artist in artists.iter_mut() {
        for album in artist.albums.iter_mut() {
            if let Some(changed) = only_changed {
                let has_changed = album.tracks.iter().any(|t| changed.contains(&t.path));
                if !has_changed {
                    continue;
                }
            }
            if album.cover_file.is_some() {
                continue;
            }
            // Tier 1：文件系统缓存
            if load_cached {
                for t in &album.tracks {
                    if existing.contains(&t.id) {
                        if let Some(p) = covers::track_cover_path(&t.id) {
                            album.cover_file = Some(p);
                            break;
                        }
                    }
                }
                if album.cover_file.is_some() {
                    continue;
                }
            }
            // Tier 2：外部封面（专辑目录向上回溯 ≤3 级；hop0 允许任意图片）
            if let Some(dir_path) = album.dir_path.clone() {
                let mut dir: Option<String> = Some(dir_path);
                let mut hop = 0;
                while hop < 3 {
                    let Some(d) = dir else { break };
                    let external = covers::find_external_cover(&d, 0)
                        .or_else(|| if hop == 0 { covers::find_any_image(&d) } else { None });
                    if let Some(src) = external {
                        if let Some(first) = album.tracks.first() {
                            if let Some(saved) = covers::save_cover_file(&first.id, &src) {
                                album.cover_file = Some(saved);
                            }
                        }
                        break;
                    }
                    dir = Path::new(&d).parent().map(|p| p.to_string_lossy().into_owned());
                    if dir.as_deref() == Some(d.as_str()) {
                        break;
                    }
                    hop += 1;
                }
                if album.cover_file.is_some() {
                    continue;
                }
            }
            // Tier 3：内嵌封面（重新解析专辑第一轨）
            if let Some(first) = album.tracks.first() {
                let m = parse_file(&first.path);
                if let Some(c) = m.cover {
                    album.cover_file = Some(c);
                }
            }
        }
    }
}

/// 封面落盘：专辑封面复制给专辑内所有音轨；文件夹封面写入 folder_{hash}.jpg + 映射
fn save_covers(artists: &[ScannedArtist], tree: &[FolderNode], changed_only: Option<&HashSet<String>>) -> (usize, usize) {
    let mut track_saved = 0usize;
    let mut track_total = 0usize;
    for artist in artists {
        for album in &artist.albums {
            let Some(cover_file) = &album.cover_file else { continue };
            for t in &album.tracks {
                if let Some(changed) = changed_only {
                    if !changed.contains(&t.path) {
                        continue;
                    }
                }
                track_total += 1;
                if covers::track_cover_path(&t.id).is_none() && covers::copy_track_cover(
                    Path::new(cover_file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default().as_str(),
                    &t.id,
                ) {
                    track_saved += 1;
                }
            }
        }
    }

    let mut folder_saved = 0usize;
    let mut folder_total = 0usize;
    fn save_folder_covers(
        node: &FolderNode,
        changed_folders: Option<&HashSet<String>>,
        folder_saved: &mut usize,
        folder_total: &mut usize,
    ) {
        let np = normalize_path(&node.path);
        let affected = changed_folders.is_none_or(|s| s.contains(&np));
        if affected {
            let mut src = covers::external_cover_in_dir(&np);
            if src.is_none() {
                for t in &node.tracks {
                    if let Some(p) = covers::track_cover_path(&t.id) {
                        src = Some(PathBuf::from(p));
                        break;
                    }
                }
            }
            if src.is_none() {
                for child in &node.children {
                    if let Some(p) = covers::folder_cover_path(&normalize_path(&child.path)) {
                        src = Some(PathBuf::from(p));
                        break;
                    }
                }
            }
            if let Some(src) = src {
                *folder_total += 1;
                let hash = covers::cache_key(&np);
                let saved = covers::save_cover_file(&format!("folder_{hash}"), &src);
                if saved.is_some() || covers::folder_cover_path(&np).is_some() {
                    covers::set_folder_cover_mapping(&np);
                    *folder_saved += 1;
                }
            }
        }
        for child in &node.children {
            save_folder_covers(child, changed_folders, folder_saved, folder_total);
        }
    }
    for r in tree {
        save_folder_covers(r, changed_only.map(|s| {
            // 变更文件的所有祖先目录
            let mut set = HashSet::new();
            for p in s {
                let np = normalize_path(p);
                if let Some(i) = np.rfind('/') {
                    set.insert(np[..i].to_string());
                }
            }
            set
        }).as_ref(), &mut folder_saved, &mut folder_total);
    }
    (track_saved + folder_saved, track_total + folder_total)
}

fn to_dto_artists(artists: Vec<ScannedArtist>) -> Vec<Artist> {
    artists
        .into_iter()
        .map(|a| Artist {
            name: a.name,
            path: a.path,
            albums: a
                .albums
                .into_iter()
                .map(|al| Album {
                    name: al.name,
                    artist: al.artist,
                    cover_path: None,
                    cover_data: None,
                    tracks: al.tracks,
                })
                .collect(),
        })
        .collect()
}

/// 全量扫描（含保存快照 + 封面）
pub fn scan_full(app: &AppHandle, folder_paths: &[String], db_path: &Path) -> ScanResult {
    crate::paths::append_log(&format!("[scan] full scan folders: {folder_paths:?}"));
    emit_stage(app, "发现文件...");
    let files = discover_files(folder_paths);
    let total = files.len() as i64;
    emit_progress(app, 0, total);
    emit_stage(app, &format!("解析元数据... ({total} 个文件)"));

    let metas = parse_files_parallel(&files, app, total, 0);

    emit_stage(app, "整理结构...");
    let mut artists = group_tracks(&files, &metas, folder_paths);
    let tree = build_folder_tree(&files, &metas, folder_paths);
    fill_album_covers(&mut artists, None, true);
    let _ = save_covers(&artists, &tree, None);

    let all_tracks: Vec<Track> = artists.iter().flat_map(|a| a.albums.iter().flat_map(|al| al.tracks.clone())).collect();
    let result = ScanResult {
        folder_paths: folder_paths.iter().map(|p| normalize_path(p)).collect(),
        artists: to_dto_artists(artists),
        folder_tree: tree,
        all_tracks: all_tracks.clone(),
        file_count: files.len() as i64,
        scanned_at: now_ms(),
    };
    persist_snapshot(db_path, &result);
    result
}

/// 增量扫描：只解析新/变化文件。
/// 返回 (结果, 是否有实际变更)；无变更时完全不发出 stage/progress 事件，
/// 避免启动时在 UI 上闪出「构建音乐库…」与扫描转圈。
pub fn scan_incremental_quiet(app: &AppHandle, folder_paths: &[String], db_path: &Path) -> (ScanResult, bool) {
    crate::paths::append_log(&format!("[scan-incr] folders: {folder_paths:?}"));
    let full_meta = db::library::load_full_meta_index(db_path);
    let files = discover_files(folder_paths);

    let mut changed: Vec<String> = Vec::new();
    for f in &files {
        let np = normalize_path(f);
        match full_meta.get(&np) {
            None => changed.push(f.clone()),
            Some(c) => {
                let stat = stat_file(f);
                let refresh_video = is_video(f) && c.duration <= 0.0;
                // mtime 容差 2ms：Node 存的是带亚毫秒精度的 float，Rust 截断为整毫秒
                let unchanged = stat.is_some_and(|(mtime, size)| {
                    (mtime - c.file_mtime).abs() <= 2.0 && size == c.file_size
                });
                if refresh_video || !unchanged {
                    changed.push(f.clone());
                }
            }
        }
    }
    let has_changes = !changed.is_empty() || files.len() != full_meta.len();
    crate::paths::append_log(&format!("[scan-incr] {}/{} new/changed", changed.len(), files.len()));

    if !has_changes {
        // 无变化：直接返回快照，不发任何 UI 事件（启动路径静默）
        if let Some(snapshot) = db::library::load_snapshot(db_path, true) {
            return (snapshot, false);
        }
    }

    let total = files.len() as i64;
    emit_progress(app, 0, total);
    emit_stage(app, &format!("解析元数据... ({total} 个文件)"));

    let mut metas: HashMap<String, FileMeta> = HashMap::new();
    for f in &files {
        let np = normalize_path(f);
        if let Some(c) = full_meta.get(&np) {
            if !changed.contains(f) {
                metas.insert(
                    f.clone(),
                    FileMeta {
                        duration: c.duration,
                        title: c.title.clone(),
                        artist: c.artist.clone(),
                        genre: c.genre.clone(),
                        bitrate: c.bitrate,
                        sample_rate: c.sample_rate,
                    },
                );
            }
        }
    }
    let changed_set: HashSet<String> = changed.iter().map(|p| normalize_path(p)).collect();
    if !changed.is_empty() {
        emit_stage(app, "解析新文件元数据...");
        let parsed = parse_files_parallel(&changed, app, total, total - changed.len() as i64);
        metas.extend(parsed);
    }

    emit_stage(app, "构建音乐库...");
    let mut artists = group_tracks(&files, &metas, folder_paths);
    let tree = build_folder_tree(&files, &metas, folder_paths);
    fill_album_covers(&mut artists, Some(&changed_set), false);
    let _ = save_covers(&artists, &tree, Some(&changed_set));

    let all_tracks: Vec<Track> = artists.iter().flat_map(|a| a.albums.iter().flat_map(|al| al.tracks.clone())).collect();
    let result = ScanResult {
        folder_paths: folder_paths.iter().map(|p| normalize_path(p)).collect(),
        artists: to_dto_artists(artists),
        folder_tree: tree,
        all_tracks: all_tracks.clone(),
        file_count: files.len() as i64,
        scanned_at: now_ms(),
    };
    persist_snapshot(db_path, &result);
    (result, true)
}

/// 兼容入口：需要变更标志时用 scan_incremental_quiet。
pub fn scan_incremental(app: &AppHandle, folder_paths: &[String], db_path: &Path) -> ScanResult {
    scan_incremental_quiet(app, folder_paths, db_path).0
}

fn persist_snapshot(db_path: &Path, result: &ScanResult) {
    if let Err(e) = db::library::save_snapshot(db_path, result) {
        crate::paths::append_log(&format!("[scan] saveLibrary failed: {e}"));
    }
    db::progress::prune_progress_and_bookmarks(db_path);
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// 移除扫描根目录：从既有快照重建（不触发全量重扫）
pub fn remove_folder(app: &AppHandle, folder_path: &str, remaining: &[String], db_path: &Path) -> ScanResult {
    let snapshot = match db::library::load_snapshot(db_path, false) {
        Some(s) => s,
        None => {
            return ScanResult {
                folder_paths: remaining.iter().map(|p| normalize_path(p)).collect(),
                ..Default::default()
            }
        }
    };
    let np = normalize_path(folder_path);
    let kept: Vec<Track> = snapshot
        .all_tracks
        .into_iter()
        .filter(|t| {
            let tp = normalize_path(&t.path);
            tp != np && !tp.starts_with(&format!("{np}/"))
        })
        .collect();

    // 移除条目的封面文件
    let covers_dir = crate::paths::covers_dir();
    {
        let old_tracks = db::library::load_snapshot(db_path, true)
            .map(|s| s.all_tracks)
            .unwrap_or_default();
        for t in old_tracks {
            let tp = normalize_path(&t.path);
            if tp == np || tp.starts_with(&format!("{np}/")) {
                let _ = std::fs::remove_file(covers_dir.join(format!("{}.jpg", t.id)));
            }
        }
    }

    let remaining_norm: Vec<String> = remaining.iter().map(|p| normalize_path(p)).collect();
    let mut metas: HashMap<String, FileMeta> = HashMap::new();
    let files: Vec<String> = kept.iter().map(|t| t.path.clone()).collect();
    for t in &kept {
        metas.insert(
            t.path.clone(),
            FileMeta {
                duration: t.duration,
                title: t.meta_title.clone(),
                artist: t.meta_artist.clone(),
                genre: t.genre.clone(),
                bitrate: t.bitrate,
                sample_rate: t.sample_rate,
            },
        );
    }

    emit_stage(app, "重建曲库...");
    let artists = group_tracks(&files, &metas, &remaining_norm);
    let tree = build_folder_tree(&files, &metas, &remaining_norm);
    let result = ScanResult {
        folder_paths: remaining_norm,
        artists: to_dto_artists(artists),
        folder_tree: tree,
        all_tracks: kept.clone(),
        file_count: kept.len() as i64,
        scanned_at: now_ms(),
    };
    persist_snapshot(db_path, &result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("C:/music/track{i}.mp3")).collect()
    }

    fn noop_report() -> Arc<dyn Fn(usize) + Send + Sync> {
        Arc::new(|_| {})
    }

    fn meta_for(p: &str, duration: f64) -> ScanMeta {
        ScanMeta { duration, title: Some(basename_no_ext(p)), ..Default::default() }
    }

    #[test]
    fn pool_parses_every_file_exactly_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = Arc::clone(&calls);
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> =
            Arc::new(move |p: &str| {
                calls_in.fetch_add(1, Ordering::Relaxed);
                meta_for(p, 1.5)
            });
        let meta = parse_pool(&paths(500), PARSE_TIMEOUT, noop_report(), parse);
        assert_eq!(meta.len(), 500);
        assert_eq!(calls.load(Ordering::Relaxed), 500, "each file parsed exactly once");
        assert!(meta.values().all(|m| m.duration == 1.5));
    }

    #[test]
    fn pool_handles_empty_input() {
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> = Arc::new(|_| ScanMeta::default());
        let meta = parse_pool(&[], PARSE_TIMEOUT, noop_report(), parse);
        assert!(meta.is_empty());
    }

    #[test]
    fn pool_small_library_still_completes() {
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> =
            Arc::new(|p: &str| meta_for(p, 2.0));
        let meta = parse_pool(&paths(3), PARSE_TIMEOUT, noop_report(), parse);
        assert_eq!(meta.len(), 3);
        assert_eq!(meta["C:/music/track0.mp3"].title.as_deref(), Some("track0"));
    }

    #[test]
    fn pool_panicking_parse_falls_back_without_losing_the_file() {
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> = Arc::new(|p: &str| {
            if p.ends_with("track7.mp3") {
                panic!("boom on track7");
            }
            meta_for(p, 9.0)
        });
        let meta = parse_pool(&paths(20), PARSE_TIMEOUT, noop_report(), parse);
        assert_eq!(meta.len(), 20, "panicking file still yields a fallback entry");
        let fallback = &meta["C:/music/track7.mp3"];
        assert_eq!(fallback.duration, 0.0);
        assert_eq!(fallback.title.as_deref(), Some("track7"));
        assert_eq!(meta["C:/music/track0.mp3"].duration, 9.0);
    }

    #[test]
    fn pool_enforces_per_file_timeout_with_fallback_meta() {
        // 第一个文件卡住远超超时，其余文件必须照常完成
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> = Arc::new(|p: &str| {
            if p.ends_with("track0.mp3") {
                std::thread::sleep(Duration::from_millis(400));
            }
            meta_for(p, 3.0)
        });
        let meta = parse_pool(&paths(4), Duration::from_millis(80), noop_report(), parse);
        assert_eq!(meta.len(), 4, "all files counted despite one timeout");
        let timed_out = &meta["C:/music/track0.mp3"];
        assert_eq!(timed_out.duration, 0.0);
        assert_eq!(timed_out.title.as_deref(), Some("track0"));
    }

    #[test]
    fn pool_reports_progress_up_to_total() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let seen_in = Arc::clone(&seen);
        let report: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |done: usize| {
            seen_in.lock().unwrap_or_else(|e| e.into_inner()).push(done);
        });
        let parse: Arc<dyn Fn(&str) -> ScanMeta + Send + Sync> =
            Arc::new(|p: &str| meta_for(p, 1.0));
        let meta = parse_pool(&paths(50), PARSE_TIMEOUT, report, parse);
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(meta.len(), 50);
        assert_eq!(seen.first().copied(), Some(0), "progress starts at 0");
        assert_eq!(seen.last().copied(), Some(50), "progress ends at total");
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "progress is monotonic");
    }

    #[test]
    fn timeout_meta_uses_basename_without_extension() {
        let m = timeout_meta("C:/music/Album/Track 01.flac");
        assert_eq!(m.duration, 0.0);
        assert_eq!(m.title.as_deref(), Some("Track 01"));
        assert!(m.cover.is_none());
        assert!(m.artist.is_none());
    }
}
