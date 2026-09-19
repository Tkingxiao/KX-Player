//! 扫描编排：文件发现 → 并行元数据解析 → 结构分组 → 封面落盘 → 快照入库。
//! 与 Electron fileScanner.ts 的算法逐段对齐（含三层封面、祖先链文件夹树、增量判据）。

pub mod covers;
pub mod meta;

use crate::db;
use crate::model::{normalize_path, Album, Artist, FolderNode, ScanResult, Track};
use crate::scanner::meta::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

const MAX_DIR_COUNT: usize = 10000;

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
    while let Some(dir) = stack.pop() {
        if visited.contains(&dir) || visited.len() >= MAX_DIR_COUNT {
            continue;
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

/// 并行解析（scoped threads，worker 数与 Electron 版一致：min(4, max(2, cpu*0.75))），
/// 进度每 120ms 节流上报。
fn parse_files_parallel(files: &[String], app: &AppHandle, total_hint: i64, done_offset: i64) -> HashMap<String, FileMeta> {
    let total = files.len();
    let counter = Arc::new(AtomicUsize::new(0));
    let results: Arc<std::sync::Mutex<HashMap<String, FileMeta>>> = Arc::new(std::sync::Mutex::new(HashMap::new()));

    if total == 0 {
        emit_progress(app, done_offset, total_hint);
        return HashMap::new();
    }

    let threads = std::cmp::min(4, std::cmp::max(2, std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)));
    let chunk_size = total.div_ceil(threads);

    std::thread::scope(|scope| {
        // 进度上报线程
        {
            let counter = Arc::clone(&counter);
            let app = app.clone();
            scope.spawn(move || loop {
                let done = counter.load(Ordering::Relaxed);
                emit_progress(&app, done_offset + done as i64, total_hint);
                if done >= total {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            });
        }
        for chunk in files.chunks(chunk_size) {
            let counter = Arc::clone(&counter);
            let results = Arc::clone(&results);
            scope.spawn(move || {
                for f in chunk {
                    let meta = parse_file(f);
                    results.lock().unwrap_or_else(|e| e.into_inner()).insert(
                        f.clone(),
                        FileMeta {
                            duration: meta.duration,
                            title: meta.title,
                            artist: meta.artist,
                            genre: meta.genre,
                            bitrate: meta.bitrate,
                            sample_rate: meta.sample_rate,
                        },
                    );
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    emit_progress(app, done_offset + total as i64, total_hint);
    let out = results.lock().unwrap_or_else(|e| e.into_inner()).clone();
    out
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
                album_cover_data: None,
            });
        }
        // 祖先链：确保从叶子目录到根的所有中间目录节点存在
        // （children 挂接统一由后方的排序链接循环完成，父目录路径必然先于子目录处理）
        let mut cur = dir.clone();
        loop {
            let Some(parent_end) = cur.rfind('/') else { break };
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
        node.children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        node.tracks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        node
    }

    let mut roots_sorted: Vec<FolderNode> = Vec::new();
    for rp in &root_paths {
        roots_sorted.push(assemble(rp, &mut map, &children_map));
    }

    // 排序 + 计数
    fn finalize(node: &mut FolderNode) -> i64 {
        node.children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        node.tracks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
    roots_sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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

/// 封面落盘：专辑封面复制给专辑内所有音轨；文件夹封面写入 folder_{md5}.jpg + 映射
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
                let hash = covers::md5_hex(&np);
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

/// 增量扫描：只解析新/变化文件
pub fn scan_incremental(app: &AppHandle, folder_paths: &[String], db_path: &Path) -> ScanResult {
    crate::paths::append_log(&format!("[scan-incr] folders: {folder_paths:?}"));
    let full_meta = db::library::load_full_meta_index(db_path);
    emit_stage(app, "发现文件...");
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
    crate::paths::append_log(&format!("[scan-incr] {}/{} new/changed", changed.len(), files.len()));
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
    result
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
