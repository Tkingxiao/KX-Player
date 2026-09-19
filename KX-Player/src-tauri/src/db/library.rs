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
                t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate
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
                    for (_, (aid, album)) in albums_by_id.iter() {
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
            node.children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            node.tracks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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

/// 事务内全量重建（better-sqlite3 版同款语义：单事务，中断回滚）
pub fn save_snapshot(db_path: &Path, snapshot: &ScanResult) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM folder_tracks;
             DELETE FROM folder_nodes;
             DELETE FROM tracks;
             DELETE FROM albums;
             DELETE FROM artists;
             DELETE FROM library_meta;",
        )?;
        {
            let mut ins_meta = tx.prepare("INSERT INTO library_meta (key, value) VALUES (?1, ?2)")?;
            ins_meta.execute(rusqlite::params!["folderPaths", serde_json::to_string(&snapshot.folder_paths).unwrap_or_default()])?;
            ins_meta.execute(rusqlite::params!["fileCount", snapshot.file_count.to_string()])?;
            ins_meta.execute(rusqlite::params!["scannedAt", snapshot.scanned_at.to_string()])?;
        }
        {
            let mut ins_artist = tx.prepare("INSERT INTO artists (name, root_path) VALUES (?1, ?2)")?;
            let mut ins_album = tx.prepare(
                "INSERT INTO albums (artist_id, name, artist_name, cover_path, cover_data) VALUES (?1, ?2, ?3, ?4, NULL)",
            )?;
            let mut ins_track = tx.prepare(
                "INSERT INTO tracks (
                    id, artist_id, album_id, name, path, duration, artist, album, format, is_video,
                    cover_path, cover_data, lyrics_path, file_mtime, file_size, meta_title, meta_artist,
                    genre, bitrate, sample_rate, album_cover_data
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13,?14,?15,?16,?17,?18,?19,NULL)",
            )?;
            for artist in &snapshot.artists {
                ins_artist.execute(rusqlite::params![artist.name, normalize_root(&artist.path)])?;
                let artist_id = tx.last_insert_rowid();
                for album in &artist.albums {
                    ins_album.execute(rusqlite::params![artist_id, album.name, album.artist, album.cover_path])?;
                    let album_id = tx.last_insert_rowid();
                    for t in &album.tracks {
                        ins_track.execute(rusqlite::params![
                            t.id, artist_id, album_id, t.name, normalize_root(&t.path),
                            t.duration as i64, t.artist, t.album, t.format, t.is_video as i64,
                            t.cover_path, t.lyrics_path, t.file_mtime, t.file_size,
                            t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate,
                        ])?;
                    }
                }
            }
        }
        {
            let mut ins_folder = tx.prepare(
                "INSERT INTO folder_nodes (path, name, parent_path, track_count, cover_data) VALUES (?1, ?2, ?3, ?4, NULL)",
            )?;
            let mut ins_folder_track = tx.prepare("INSERT OR IGNORE INTO folder_tracks (folder_path, track_id) VALUES (?1, ?2)")?;
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
                parents.entry(np.clone()).or_insert(None);
            }
            let mut stack: Vec<&FolderNode> = snapshot.folder_tree.iter().collect();
            let mut si = 0;
            while si < stack.len() {
                let node = stack[si];
                si += 1;
                let np = normalize_root(&node.path);
                ins_folder.execute(rusqlite::params![np, node.name, parents.get(&np).cloned().flatten(), node.track_count])?;
                for t in &node.tracks {
                    ins_folder_track.execute(rusqlite::params![np, t.id])?;
                }
                stack.extend(node.children.iter());
            }
        }
        tx.commit()
    })
}

fn normalize_root(p: &str) -> String {
    crate::model::normalize_path(p)
}

/// 扫描用元数据索引：path → (duration, hasCover, title, artist, mtime, size, genre, bitrate, sampleRate)

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
