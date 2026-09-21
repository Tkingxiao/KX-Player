//! 曲库快照读写（封面 base64 不再存 DB，全部走文件系统；schema 与 Electron 版一致）。

use super::{initialize_schema, with_db};
use crate::model::{Album, Artist, FolderNode, ScanResult, Track};
use rusqlite::{Connection, Result};
use std::collections::HashMap;
use std::path::Path;

struct TrackRow {
    album_id: i64,
    track: Track,
}

fn load_track_rows(conn: &Connection) -> Result<Vec<TrackRow>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.album_id, t.name, t.path, t.duration, t.artist, t.album,
                t.format, t.is_video, t.cover_path, t.lyrics_path, t.file_mtime, t.file_size,
                t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate, t.loudness_lufs
         FROM tracks t
         ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(TrackRow {
            album_id: r.get(1)?,
            track: Track {
                id: r.get(0)?,
                name: r.get(2)?,
                path: r.get(3)?,
                duration: r.get::<_, i64>(4).unwrap_or(0) as f64,
                artist: r.get(5)?,
                album: r.get(6)?,
                format: r.get(7)?,
                is_video: r.get::<_, i64>(8).unwrap_or(0) == 1,
                cover_path: r.get(9)?,
                cover_data: None,
                lyrics_path: r.get(10)?,
                file_mtime: r.get::<_, f64>(11).unwrap_or(0.0),
                file_size: r.get::<_, i64>(12).unwrap_or(0),
                meta_title: r.get(13)?,
                meta_artist: r.get(14)?,
                genre: r.get(15)?,
                bitrate: r.get(16)?,
                sample_rate: r.get(17)?,
                loudness_lufs: r.get(18)?,
                album_cover_data: None,
            },
        })
    })?;
    rows.collect()
}

fn meta_string(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM library_meta WHERE key = ?1", [key], |r| r.get(0)).ok()
}

/// lean=true 时不构建 artists/albums 结构（快速启动路径，省去 join 开销）
pub fn load_snapshot(db_path: &Path, lean: bool) -> Option<ScanResult> {
    if !db_path.exists() {
        return None;
    }
    let loaded = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let raw = meta_string(conn, "folderPaths").ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let folder_paths: Vec<String> = serde_json::from_str(&raw).map_err(|_| rusqlite::Error::QueryReturnedNoRows)?;

        let track_rows = load_track_rows(conn)?;
        let mut tracks_by_album: HashMap<i64, Vec<Track>> = HashMap::new();
        let mut all_tracks: Vec<Track> = Vec::with_capacity(track_rows.len());
        let mut tracks_by_id: HashMap<String, Track> = HashMap::new();
        for row in track_rows {
            tracks_by_album.entry(row.album_id).or_default().push(row.track.clone());
            tracks_by_id.insert(row.track.id.clone(), row.track.clone());
            all_tracks.push(row.track);
        }

        let artists: Vec<Artist> = if lean {
            Vec::new()
        } else {
            let mut albums_by_id: HashMap<i64, (i64, Album)> = HashMap::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT album_id, artist_id, name, artist_name, cover_path FROM albums
                     ORDER BY artist_name COLLATE NOCASE, name COLLATE NOCASE",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        Album {
                            name: r.get(2)?,
                            artist: r.get(3)?,
                            cover_path: r.get(4)?,
                            cover_data: None,
                            tracks: Vec::new(),
                        },
                    ))
                })?;
                for row in rows {
                    let (album_id, artist_id, album) = row?;
                    albums_by_id.insert(album_id, (artist_id, album));
                }
            }
            for (album_id, tracks) in tracks_by_album {
                if let Some((_, album)) = albums_by_id.get_mut(&album_id) {
                    album.tracks = tracks;
                }
            }
            let mut out: Vec<Artist> = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT a.artist_id, a.name, a.root_path FROM artists a ORDER BY a.name COLLATE NOCASE",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })?;
                for row in rows {
                    let (artist_id, name, path) = row?;
                    let mut albums: Vec<Album> = Vec::new();
                    for (aid, album) in albums_by_id.values() {
                        if *aid == artist_id {
                            albums.push(album.clone());
                        }
                    }
                    out.push(Artist { name, path, albums });
                }
            }
            out
        };

        // 文件夹行 → 树
        let mut folder_rows: Vec<(String, String, Option<String>, i64)> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT path, name, parent_path, track_count FROM folder_nodes ORDER BY path COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for row in rows {
                folder_rows.push(row?);
            }
        }

        let mut node_map: HashMap<String, FolderNode> = HashMap::new();
        for (path, name, _parent_path, track_count) in &folder_rows {
            node_map.insert(
                path.clone(),
                FolderNode {
                    name: name.clone(),
                    path: path.clone(),
                    children: Vec::new(),
                    tracks: Vec::new(),
                    track_count: *track_count,
                    cover_data: None,
                },
            );
        }
        // 挂接音轨
        for (folder_path, track_id) in folder_track_rows(conn)? {
            if let Some(node) = node_map.get_mut(&folder_path) {
                if let Some(track) = tracks_by_id.get(&track_id) {
                    node.tracks.push(track.clone());
                }
            }
        }
        // 组树：parent_path 在库中的挂到父节点，否则为根
        // （先建关系表再递归组装，避免父节点被移出 map 后子节点失联成孤儿根）
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut root_keys: Vec<String> = Vec::new();
        for (path, _, parent_path, _) in &folder_rows {
            match parent_path {
                Some(p) if node_map.contains_key(p) => children_map.entry(p.clone()).or_default().push(path.clone()),
                _ => root_keys.push(path.clone()),
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
        let mut roots: Vec<FolderNode> = Vec::new();
        for key in &root_keys {
            roots.push(assemble(key, &mut node_map, &children_map));
        }

        let file_count: i64 = meta_string(conn, "fileCount")
            .and_then(|v| v.parse().ok())
            .unwrap_or(all_tracks.len() as i64);

        Ok(ScanResult {
            folder_paths,
            artists,
            folder_tree: roots,
            all_tracks,
            file_count,
            scanned_at: 0,
        })
    });
    loaded.ok()
}

fn folder_track_rows(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT folder_path, track_id FROM folder_tracks")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

/// 快照落库：**与库中现状比对，只写差异**。
///
/// 旧实现是单事务内 `DELETE` 六张表再全量重插，正是 `01 §4.1` 点名的禁止项：
/// 中断即丢库、且增量扫描的写放大随库线性增长。差异写入把每次落库压到「真正动过的行」，
/// 顺带消掉旧代码为躲开重插而预读 `loudness_lufs` 再回填的那段补丁。
///
/// 仍在单事务里：失败整体回滚，但不再有「表已删空、还没插回来」的窗口。
/// 顺序按外键排：artist/album 先补（新曲目要能被引用）→ tracks upsert →
/// folder_nodes upsert → folder_tracks 先清失效再补缺口 → 删消失的 track 与 folder →
/// 最后回收不再被任何曲目引用的 album/artist。
pub fn save_snapshot(db_path: &Path, snapshot: &ScanResult) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let tx = conn.unchecked_transaction()?;
        let stats = write_diff(&tx, snapshot)?;
        tx.commit()?;
        if stats.touched() {
            crate::paths::append_log(&format!(
                "[scan] 差异写入：曲目 +{}/~{}/-{}，目录 +{}/-{}，归属 +{}/-{}",
                stats.track_ins, stats.track_upd, stats.track_del, stats.folder_ins, stats.folder_del, stats.pair_ins, stats.pair_del
            ));
        }
        Ok(())
    })
}

/// 一次差异写入的计数。`touched()` 为 false 就是「增量扫描什么都没动」。
#[derive(Default)]
struct WriteStats {
    track_ins: usize,
    track_upd: usize,
    track_del: usize,
    folder_ins: usize,
    folder_del: usize,
    pair_ins: usize,
    pair_del: usize,
}

impl WriteStats {
    fn touched(&self) -> bool {
        self.track_ins + self.track_upd + self.track_del + self.folder_ins + self.folder_del + self.pair_ins + self.pair_del > 0
    }
}

/// 落库列的可比对投影。**`duration` 存 INTEGER**，故两侧都按 `as i64` 归一，
/// 否则 3.0 与 3 会被判成「变了」，每次扫描全库重写。
/// `loudness_lufs` 故意不在这里：它由 `set_track_loudness` 单独维护，不该算作快照差异。
#[derive(PartialEq)]
struct TrackSig {
    artist_id: i64,
    album_id: i64,
    name: String,
    path: String,
    duration: i64,
    artist: String,
    album: String,
    format: String,
    is_video: bool,
    cover_path: Option<String>,
    lyrics_path: Option<String>,
    file_mtime: f64,
    file_size: i64,
    meta_title: Option<String>,
    meta_artist: Option<String>,
    genre: Option<String>,
    bitrate: Option<i64>,
    sample_rate: Option<i64>,
}

fn sig_of(t: &Track, artist_id: i64, album_id: i64) -> TrackSig {
    TrackSig {
        artist_id,
        album_id,
        name: t.name.clone(),
        path: normalize_root(&t.path),
        duration: t.duration as i64,
        artist: t.artist.clone(),
        album: t.album.clone(),
        format: t.format.clone(),
        is_video: t.is_video,
        cover_path: t.cover_path.clone(),
        lyrics_path: t.lyrics_path.clone(),
        file_mtime: t.file_mtime,
        file_size: t.file_size,
        meta_title: t.meta_title.clone(),
        meta_artist: t.meta_artist.clone(),
        genre: t.genre.clone(),
        bitrate: t.bitrate,
        sample_rate: t.sample_rate,
    }
}

/// 库中现状的一次性快照。表都不大（万级），全读进内存换一遍 O(1) 比对。
#[derive(Default)]
struct Stored {
    /// (name, root_path) → artist_id
    artists: HashMap<(String, String), i64>,
    /// (artist_id, name) → (album_id, artist_name, cover_path)
    albums: HashMap<(i64, String), (i64, String, Option<String>)>,
    /// track id → (投影, 已存响度)
    tracks: HashMap<String, TrackSig>,
    /// folder path → (name, parent_path, track_count)
    folders: HashMap<String, (String, String, i64)>,
    folder_tracks: std::collections::HashSet<(String, String)>,
}

impl Stored {
    fn load(conn: &Connection) -> Result<Stored> {
        let mut s = Stored::default();
        {
            let mut stmt = conn.prepare("SELECT artist_id, name, root_path FROM artists")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(0)?)))?.collect::<Result<Vec<_>>>()?;
            for (name, root, id) in rows {
                s.artists.insert((name, root), id);
            }
        }
        {
            let mut stmt = conn.prepare("SELECT album_id, artist_id, name, artist_name, cover_path FROM albums")?;
            let rows = stmt.query_map([], |r| {
                Ok(((r.get::<_, i64>(1)?, r.get::<_, String>(2)?), (r.get::<_, i64>(0)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?)))
            })?
            .collect::<Result<Vec<_>>>()?;
            for (k, v) in rows {
                s.albums.insert(k, v);
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT id, artist_id, album_id, name, path, duration, artist, album, format, is_video,
                        cover_path, lyrics_path, file_mtime, file_size, meta_title, meta_artist, genre,
                        bitrate, sample_rate FROM tracks",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        TrackSig {
                            artist_id: r.get(1)?,
                            album_id: r.get(2)?,
                            name: r.get(3)?,
                            path: r.get(4)?,
                            duration: r.get::<_, i64>(5)?,
                            artist: r.get(6)?,
                            album: r.get(7)?,
                            format: r.get(8)?,
                            is_video: r.get::<_, i64>(9)? != 0,
                            cover_path: r.get(10)?,
                            lyrics_path: r.get(11)?,
                            file_mtime: r.get(12)?,
                            file_size: r.get(13)?,
                            meta_title: r.get(14)?,
                            meta_artist: r.get(15)?,
                            genre: r.get(16)?,
                            bitrate: r.get(17)?,
                            sample_rate: r.get(18)?,
                        },
                    ))
                })?
                .collect::<Result<Vec<_>>>()?;
            for (id, sig) in rows {
                s.tracks.insert(id, sig);
            }
        }
        {
            let mut stmt = conn.prepare("SELECT path, name, parent_path, track_count FROM folder_nodes")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2).unwrap_or_default(), r.get::<_, i64>(3)?))))?
                .collect::<Result<Vec<_>>>()?;
            for (path, v) in rows {
                s.folders.insert(path, v);
            }
        }
        {
            let mut stmt = conn.prepare("SELECT folder_path, track_id FROM folder_tracks")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>>>()?;
            s.folder_tracks = rows.into_iter().collect();
        }
        Ok(s)
    }
}

fn write_diff(conn: &Connection, snapshot: &ScanResult) -> Result<WriteStats> {
    let mut st = WriteStats::default();
    let mut stored = Stored::load(conn)?;
    let mut desired_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();

    {
        let mut ins_artist = conn.prepare("INSERT INTO artists (name, root_path) VALUES (?1, ?2)")?;
        let mut ins_album = conn.prepare("INSERT INTO albums (artist_id, name, artist_name, cover_path, cover_data) VALUES (?1, ?2, ?3, ?4, NULL)")?;
        let mut upd_album = conn.prepare("UPDATE albums SET artist_name = ?2, cover_path = ?3 WHERE album_id = ?1")?;
        let mut ins_track = conn.prepare(
            "INSERT INTO tracks (
                id, artist_id, album_id, name, path, duration, artist, album, format, is_video,
                cover_path, cover_data, lyrics_path, file_mtime, file_size, meta_title, meta_artist,
                genre, bitrate, sample_rate, loudness_lufs, album_cover_data
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13,?14,?15,?16,?17,?18,?19,?20,NULL)",
        )?;
        // loudness 走 COALESCE：快照没带就保留库里已分析出的值，不把响度分析成果洗掉。
        let mut upd_track = conn.prepare(
            "UPDATE tracks SET artist_id=?2, album_id=?3, name=?4, path=?5, duration=?6, artist=?7,
                    album=?8, format=?9, is_video=?10, cover_path=?11, lyrics_path=?12, file_mtime=?13,
                    file_size=?14, meta_title=?15, meta_artist=?16, genre=?17, bitrate=?18,
                    sample_rate=?19, loudness_lufs=COALESCE(?20, loudness_lufs) WHERE id=?1",
        )?;

        for artist in &snapshot.artists {
            let akey = (artist.name.clone(), normalize_root(&artist.path));
            let known = stored.artists.get(&akey).copied();
            let artist_id = match known {
                Some(id) => id,
                None => {
                    ins_artist.execute(rusqlite::params![akey.0.as_str(), akey.1.as_str()])?;
                    let id = conn.last_insert_rowid();
                    stored.artists.insert(akey, id);
                    id
                }
            };
            for album in &artist.albums {
                let lkey = (artist_id, album.name.clone());
                let known = stored.albums.get(&lkey).cloned();
                let album_id = match known {
                    Some((id, artist_name, cover_path)) => {
                        if artist_name != album.artist || cover_path != album.cover_path {
                            upd_album.execute(rusqlite::params![id, album.artist, album.cover_path])?;
                        }
                        id
                    }
                    None => {
                        ins_album.execute(rusqlite::params![artist_id, album.name, album.artist, album.cover_path])?;
                        let id = conn.last_insert_rowid();
                        stored.albums.insert(lkey, (id, album.artist.clone(), album.cover_path.clone()));
                        id
                    }
                };
                for t in &album.tracks {
                    let want = sig_of(t, artist_id, album_id);
                    desired_ids.insert(t.id.as_str());
                    match stored.tracks.get(&t.id) {
                        None => {
                            ins_track.execute(rusqlite::params![
                                t.id, artist_id, album_id, t.name, normalize_root(&t.path),
                                t.duration as i64, t.artist, t.album, t.format, t.is_video as i64,
                                t.cover_path, t.lyrics_path, t.file_mtime, t.file_size,
                                t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate,
                                t.loudness_lufs,
                            ])?;
                            st.track_ins += 1;
                        }
                        Some(have) if *have != want => {
                            upd_track.execute(rusqlite::params![
                                t.id, artist_id, album_id, t.name, want.path, want.duration, t.artist,
                                t.album, t.format, t.is_video as i64, t.cover_path, t.lyrics_path,
                                t.file_mtime, t.file_size, t.meta_title, t.meta_artist, t.genre,
                                t.bitrate, t.sample_rate, t.loudness_lufs,
                            ])?;
                            st.track_upd += 1;
                        }
                        Some(_) => {}
                    }
                }
            }
        }
    }

    // ── 目录树 ──
    let mut want_folders: HashMap<String, (String, String, i64)> = HashMap::new();
    let mut want_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    {
        let mut parents: HashMap<String, Option<String>> = HashMap::new();
        let mut queue: Vec<&FolderNode> = snapshot.folder_tree.iter().collect();
        let mut qi = 0;
        while qi < queue.len() {
            let node = queue[qi];
            qi += 1;
            let np = normalize_root(&node.path);
            for child in &node.children {
                parents.insert(normalize_root(&child.path), Some(np.clone()));
                queue.push(child);
            }
            parents.entry(np).or_insert(None);
        }
        let mut stack: Vec<&FolderNode> = snapshot.folder_tree.iter().collect();
        let mut si = 0;
        while si < stack.len() {
            let node = stack[si];
            si += 1;
            let np = normalize_root(&node.path);
            want_folders.insert(np.clone(), (node.name.clone(), parents.get(&np).cloned().flatten().unwrap_or_default(), node.track_count));
            want_pairs.extend(node.tracks.iter().map(|t| (np.clone(), t.id.clone())));
            stack.extend(node.children.iter());
        }
    }

    {
        let mut ins_folder = conn.prepare(
            "INSERT INTO folder_nodes (path, name, parent_path, track_count, cover_data) VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(path) DO UPDATE SET name = excluded.name, parent_path = excluded.parent_path, track_count = excluded.track_count",
        )?;
        let mut del_folder = conn.prepare("DELETE FROM folder_nodes WHERE path = ?1")?;
        let mut ins_pair = conn.prepare("INSERT OR IGNORE INTO folder_tracks (folder_path, track_id) VALUES (?1, ?2)")?;
        let mut del_pair = conn.prepare("DELETE FROM folder_tracks WHERE folder_path = ?1 AND track_id = ?2")?;
        let mut del_track = conn.prepare("DELETE FROM tracks WHERE id = ?1")?;

        for (path, want) in &want_folders {
            if stored.folders.get(path) != Some(want) {
                ins_folder.execute(rusqlite::params![path, want.0, want.1, want.2])?;
                st.folder_ins += 1;
            }
        }
        // 失效映射必须先于 track / folder 删除，否则外键拦住后面两步。
        for (path, id) in &stored.folder_tracks {
            if !want_pairs.contains(&(path.clone(), id.clone())) {
                del_pair.execute(rusqlite::params![path, id])?;
                st.pair_del += 1;
            }
        }
        for (path, id) in &want_pairs {
            if !stored.folder_tracks.contains(&(path.clone(), id.clone())) {
                ins_pair.execute(rusqlite::params![path, id])?;
                st.pair_ins += 1;
            }
        }
        for id in stored.tracks.keys() {
            if !desired_ids.contains(id.as_str()) {
                del_track.execute(rusqlite::params![id])?;
                st.track_del += 1;
            }
        }
        for path in stored.folders.keys() {
            if !want_folders.contains_key(path) {
                del_folder.execute(rusqlite::params![path])?;
                st.folder_del += 1;
            }
        }
    }

    conn.execute("DELETE FROM albums WHERE album_id NOT IN (SELECT album_id FROM tracks)", [])?;
    conn.execute("DELETE FROM artists WHERE artist_id NOT IN (SELECT artist_id FROM tracks)", [])?;

    {
        let mut ins_meta = conn.prepare("INSERT OR REPLACE INTO library_meta (key, value) VALUES (?1, ?2)")?;
        ins_meta.execute(rusqlite::params!["folderPaths", serde_json::to_string(&snapshot.folder_paths).unwrap_or_default()])?;
        ins_meta.execute(rusqlite::params!["fileCount", snapshot.file_count.to_string()])?;
        ins_meta.execute(rusqlite::params!["scannedAt", snapshot.scanned_at.to_string()])?;
    }
    Ok(st)
}

fn normalize_root(p: &str) -> String {
    crate::model::normalize_path(p)
}

/// 后台响度分析结果写回（单行 UPDATE，不动快照重建链路）
pub fn set_track_loudness(db_path: &Path, track_id: &str, lufs: f64) -> Result<bool> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let n = conn.execute(
            "UPDATE tracks SET loudness_lufs = ?1 WHERE id = ?2",
            rusqlite::params![lufs, track_id],
        )?;
        Ok(n > 0)
    })
}

/// 扫描用元数据索引的一项（[`load_full_meta_index`] 的 value）。
pub struct FullMetaEntry {
    pub duration: f64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub file_mtime: f64,
    pub file_size: i64,
}

/// 扫描用元数据索引：path → [`FullMetaEntry`]。供 `scanner` 判断「已入库的就别再解一遍」。
pub fn load_full_meta_index(db_path: &Path) -> HashMap<String, FullMetaEntry> {
    let mut out = HashMap::new();
    if !db_path.exists() {
        return out;
    }
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let mut stmt = conn.prepare(
            "SELECT path, duration, meta_title, meta_artist, genre, bitrate, sample_rate,
                    file_mtime, file_size FROM tracks",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1).unwrap_or(0),
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, f64>(7).unwrap_or(0.0),
                r.get::<_, i64>(8).unwrap_or(0),
            ))
        })?;
        for row in rows {
            let (path, duration, title, artist, genre, bitrate, sample_rate, file_mtime, file_size) = row?;
            out.insert(
                crate::model::normalize_path(&path),
                FullMetaEntry { duration: duration as f64, title, artist, genre, bitrate, sample_rate, file_mtime, file_size },
            );
        }
        Ok(())
    });
    out
}

#[cfg(test)]
mod tests {
    use super::write_diff;
    use crate::db::initialize_schema;
    use crate::model::{Album, Artist, FolderNode, ScanResult, Track};
    use rusqlite::Connection;

    /// 外键必须在事务外开：`PRAGMA foreign_keys` 在事务内是 no-op，
    /// 而这几条用例要守的正是「删除顺序会不会撞外键」。
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        conn.execute_batch("PRAGMA foreign_keys=ON").expect("开启外键");
        initialize_schema(&conn).expect("建表");
        conn
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .expect("计数")
    }

    fn rowid(conn: &Connection, id: &str) -> i64 {
        conn.query_row("SELECT rowid FROM tracks WHERE id = ?1", [id], |r| r.get(0))
            .unwrap_or(-1)
    }

    fn song(id: &str) -> Track {
        Track {
            id: id.into(),
            name: format!("{id} 曲目"),
            path: format!("C:/Music/AL/{id}.mp3"),
            duration: 213.0,
            artist: "A".into(),
            album: "AL".into(),
            format: "MP3".into(),
            file_mtime: 1700.0,
            file_size: 1024,
            meta_title: Some(id.into()),
            ..Default::default()
        }
    }

    /// 一个 artist / 一张专辑 / 一个根目录，把变量收敛到「曲目集合」这一维。
    fn snapshot(songs: Vec<Track>) -> ScanResult {
        let album = Album {
            name: "AL".into(),
            artist: "A".into(),
            tracks: songs.clone(),
            ..Default::default()
        };
        let node = FolderNode {
            name: "Music".into(),
            path: "C:/Music".into(),
            track_count: songs.len() as i64,
            tracks: songs.clone(),
            ..Default::default()
        };
        ScanResult {
            folder_paths: vec!["C:/Music".into()],
            artists: vec![Artist {
                name: "A".into(),
                path: "C:/Music".into(),
                albums: vec![album],
            }],
            folder_tree: vec![node],
            all_tracks: songs.clone(),
            file_count: songs.len() as i64,
            scanned_at: 1,
        }
    }

    #[test]
    fn first_write_inserts_every_row() {
        let conn = db();
        let st = write_diff(&conn, &snapshot(vec![song("t1"), song("t2")])).expect("首次写入");
        assert_eq!((st.track_ins, st.track_upd, st.track_del), (2, 0, 0));
        assert_eq!(count(&conn, "artists"), 1);
        assert_eq!(count(&conn, "albums"), 1);
        assert_eq!(count(&conn, "folder_nodes"), 1);
        assert_eq!(count(&conn, "folder_tracks"), 2);
    }

    /// B2 的红线：同一份快照再写一遍必须一行都不动，且曲目行不被删后重插。
    /// 旧实现是全表清空重建，每次增量扫描都把整库重写一遍。
    #[test]
    fn rewriting_the_same_snapshot_writes_nothing() {
        let conn = db();
        write_diff(&conn, &snapshot(vec![song("t1"), song("t2")])).expect("播种");
        let before = (rowid(&conn, "t1"), rowid(&conn, "t2"));

        let st = write_diff(&conn, &snapshot(vec![song("t1"), song("t2")])).expect("再写");
        assert_eq!(
            (
                st.track_ins, st.track_upd, st.track_del, //
                st.folder_ins, st.folder_del,              //
                st.pair_ins, st.pair_del,
            ),
            (0, 0, 0, 0, 0, 0, 0),
            "重复快照应当零写入"
        );
        assert!(!st.touched());
        assert_eq!(before, (rowid(&conn, "t1"), rowid(&conn, "t2")), "rowid 变了说明是删后重插");
    }

    /// `duration` 落库是 INTEGER：213.9 与 213.0 比对成相等，否则每次扫描全库 UPDATE。
    #[test]
    fn sub_second_duration_noise_does_not_rewrite_rows() {
        let conn = db();
        write_diff(&conn, &snapshot(vec![song("t1")])).expect("播种");
        let mut drifted = song("t1");
        drifted.duration = 213.9;
        let st = write_diff(&conn, &snapshot(vec![drifted])).expect("亚秒漂移");
        assert_eq!((st.track_ins, st.track_upd, st.track_del), (0, 0, 0));
    }

    /// 真变化只走 UPDATE，并且不能把 `set_track_loudness` 攒下的响度洗掉。
    #[test]
    fn changed_metadata_updates_in_place_and_keeps_loudness() {
        let conn = db();
        write_diff(&conn, &snapshot(vec![song("t1"), song("t2")])).expect("播种");
        conn.execute("UPDATE tracks SET loudness_lufs = -16.5 WHERE id = 't1'", [])
            .expect("预置响度");
        let before = rowid(&conn, "t1");

        let mut moved = song("t1");
        moved.genre = Some("Rock".into());
        moved.bitrate = Some(320);
        let st = write_diff(&conn, &snapshot(vec![moved, song("t2")])).expect("改元数据");
        assert_eq!((st.track_ins, st.track_upd, st.track_del), (0, 1, 0));
        assert_eq!(rowid(&conn, "t1"), before, "UPDATE 不该换行");

        let lufs: f64 = conn
            .query_row("SELECT loudness_lufs FROM tracks WHERE id = 't1'", [], |r| r.get(0))
            .expect("读响度");
        assert_eq!(lufs, -16.5, "快照没带响度时不能覆盖已分析结果");
        let genre: String = conn
            .query_row("SELECT genre FROM tracks WHERE id = 't1'", [], |r| r.get(0))
            .expect("读 genre");
        assert_eq!(genre, "Rock");
    }

    /// 曲目消失：先清归属再删曲目（外键顺序）；album / artist 在没人引用后回收。
    #[test]
    fn removed_tracks_are_deleted_and_orphan_groups_recycled() {
        let conn = db();
        write_diff(&conn, &snapshot(vec![song("t1"), song("t2")])).expect("播种");

        let st = write_diff(&conn, &snapshot(vec![song("t1")])).expect("删一首");
        assert_eq!((st.track_del, st.pair_del), (1, 1));
        assert_eq!(count(&conn, "folder_tracks"), 1);
        assert_eq!(count(&conn, "tracks"), 1);
        assert_eq!(count(&conn, "albums"), 1, "专辑仍有曲目，不该回收");

        let st = write_diff(&conn, &snapshot(vec![])).expect("删干净");
        assert_eq!(st.track_del, 1);
        assert_eq!(count(&conn, "tracks"), 0);
        assert_eq!(count(&conn, "albums"), 0);
        assert_eq!(count(&conn, "artists"), 0);

        let st = write_diff(&conn, &ScanResult::default()).expect("连目录树一起删");
        assert_eq!(st.folder_del, 1);
        assert_eq!(count(&conn, "folder_nodes"), 0);
    }

    /// 换根目录：旧目录连同它的归属一起消失，新目录建行，曲目原地 UPDATE。
    #[test]
    fn folder_moves_replace_nodes_without_touching_track_rows() {
        let conn = db();
        write_diff(&conn, &snapshot(vec![song("t1")])).expect("播种");
        let before = rowid(&conn, "t1");

        let mut moved = song("t1");
        moved.path = "D:/KX/t1.mp3".into();
        let mut snap = snapshot(vec![moved]);
        snap.folder_paths = vec!["D:/KX".into()];
        snap.artists[0].path = "D:/KX".into();
        snap.folder_tree[0].path = "D:/KX".into();

        let st = write_diff(&conn, &snap).expect("换根目录");
        assert_eq!((st.folder_ins, st.folder_del, st.pair_del), (1, 1, 1));
        assert_eq!(st.track_upd, 1, "path 变了要走 UPDATE");
        assert_eq!(st.track_del, 0, "同一首不能被判成删除");
        assert_eq!(rowid(&conn, "t1"), before);
        assert_eq!(count(&conn, "folder_nodes"), 1);
        assert_eq!(count(&conn, "folder_tracks"), 1);
    }
}
