//! 播放进度与书签（与 Electron progressDb.ts 语义逐条一致；≥95% 完听由前端判定）。

use super::{initialize_schema, with_db};
use crate::model::{Bookmark, PlayProgress};
use rusqlite::Result;
use std::path::Path;

pub fn get_all_progress(db_path: &Path) -> Vec<PlayProgress> {
    if !db_path.exists() {
        return vec![];
    }
    let r = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let mut stmt = conn.prepare("SELECT track_id, position_ms, completed, play_count, played_at, last_speed FROM play_progress")?;
        let rows = stmt.query_map([], |r| {
            Ok(PlayProgress {
                track_id: r.get(0)?,
                position_ms: r.get(1)?,
                completed: r.get::<_, i64>(2).unwrap_or(0) == 1,
                play_count: r.get(3)?,
                played_at: r.get(4)?,
                last_speed: r.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>>>()
    });
    r.unwrap_or_default()
}

/// completed=None 表示保留原值（播放中回写位置）
pub fn set_progress(db_path: &Path, track_id: &str, position_ms: i64, completed: Option<bool>, last_speed: Option<f64>) {
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO play_progress (track_id, position_ms, completed, play_count, played_at, last_speed)
             VALUES (?1, ?2, ?3, 1, ?4, ?5)
             ON CONFLICT(track_id) DO UPDATE SET
               position_ms = excluded.position_ms,
               completed = CASE WHEN excluded.completed < 0 THEN play_progress.completed ELSE excluded.completed END,
               play_count = play_progress.play_count + 1,
               played_at = excluded.played_at,
               last_speed = excluded.last_speed",
            rusqlite::params![
                track_id,
                position_ms.max(0),
                completed.map(|c| c as i64).unwrap_or(-1),
                now_ms(),
                last_speed
            ],
        )?;
        Ok(())
    });
}

pub fn mark_completed(db_path: &Path, track_id: &str, completed: bool) {
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO play_progress (track_id, position_ms, completed, play_count, played_at)
             VALUES (?1, 0, ?2, 1, ?3)
             ON CONFLICT(track_id) DO UPDATE SET completed = excluded.completed",
            rusqlite::params![track_id, completed as i64, now_ms()],
        )?;
        Ok(())
    });
}

pub fn clear_progress(db_path: &Path, track_id: &str) {
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("DELETE FROM play_progress WHERE track_id = ?1", [track_id])?;
        Ok(())
    });
}

/// 曲库重建后清理悬空行
pub fn prune_progress_and_bookmarks(db_path: &Path) {
    if !db_path.exists() {
        return;
    }
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("DELETE FROM play_progress WHERE track_id NOT IN (SELECT id FROM tracks)", [])?;
        conn.execute("DELETE FROM bookmarks WHERE track_id NOT IN (SELECT id FROM tracks)", [])?;
        Ok(())
    });
}

pub fn list_bookmarks(db_path: &Path, track_id: &str) -> Vec<Bookmark> {
    if !db_path.exists() {
        return vec![];
    }
    let r = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, track_id, at_ms, label, created_at FROM bookmarks WHERE track_id = ?1 ORDER BY at_ms ASC",
        )?;
        let rows = stmt.query_map([track_id], |r| {
            Ok(Bookmark {
                id: r.get(0)?,
                track_id: r.get(1)?,
                at_ms: r.get(2)?,
                label: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>>>()
    });
    r.unwrap_or_default()
}

pub fn add_bookmark(db_path: &Path, track_id: &str, at_ms: i64, label: &str) -> Option<Bookmark> {
    let id = format!("bm_{}_{}", now_ms(), rand_suffix());
    let created = now_ms();
    let r = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO bookmarks (id, track_id, at_ms, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, track_id, at_ms.max(0), label, created],
        )?;
        Ok(())
    });
    r.ok()?;
    Some(Bookmark { id, track_id: track_id.into(), at_ms: at_ms.max(0), label: label.into(), created_at: created })
}

pub fn rename_bookmark(db_path: &Path, id: &str, label: &str) {
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("UPDATE bookmarks SET label = ?1 WHERE id = ?2", rusqlite::params![label, id])?;
        Ok(())
    });
}

pub fn remove_bookmark(db_path: &Path, id: &str) {
    let _ = with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    format!("{n:08x}")
}
