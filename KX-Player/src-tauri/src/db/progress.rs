//! 播放进度与书签（与 Electron progressDb.ts 语义逐条一致；≥95% 完听由前端判定）。

use super::{initialize_schema, with_db};
use crate::model::{Bookmark, PlayProgress};
use rusqlite::Result;
use std::path::Path;

/// 库文件不存在是「还没有任何进度」的正常状态（返回空列表），不是失败；
/// 库在但读失败（权限/损坏）才是 Err —— 空与失败必须可区分（P1-6）。
pub fn get_all_progress(db_path: &Path) -> Result<Vec<PlayProgress>> {
    if !db_path.exists() {
        return Ok(vec![]);
    }
    with_db(db_path, |conn| {
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
    })
}

/// completed=None 表示保留原值（播放中回写位置）。
/// 写失败如实上抛：命令层把它变成 `IpcError` 给前端，不再在这里吞成 no-op ——
/// 否则「写失败」与「没写」在调用方看来是同一个成功（P1-6）。
pub fn set_progress(db_path: &Path, track_id: &str, position_ms: i64, completed: Option<bool>, last_speed: Option<f64>) -> Result<()> {
    with_db(db_path, |conn| {
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
    })
}

pub fn mark_completed(db_path: &Path, track_id: &str, completed: bool) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO play_progress (track_id, position_ms, completed, play_count, played_at)
             VALUES (?1, 0, ?2, 1, ?3)
             ON CONFLICT(track_id) DO UPDATE SET completed = excluded.completed",
            rusqlite::params![track_id, completed as i64, now_ms()],
        )?;
        Ok(())
    })
}

pub fn clear_progress(db_path: &Path, track_id: &str) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("DELETE FROM play_progress WHERE track_id = ?1", [track_id])?;
        Ok(())
    })
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

pub fn list_bookmarks(db_path: &Path, track_id: &str) -> Result<Vec<Bookmark>> {
    if !db_path.exists() {
        return Ok(vec![]);
    }
    with_db(db_path, |conn| {
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
    })
}

pub fn add_bookmark(db_path: &Path, track_id: &str, at_ms: i64, label: &str) -> Result<Option<Bookmark>> {
    let id = format!("bm_{}_{}", now_ms(), rand_suffix());
    let created = now_ms();
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO bookmarks (id, track_id, at_ms, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, track_id, at_ms.max(0), label, created],
        )?;
        Ok(Some(Bookmark { id, track_id: track_id.into(), at_ms: at_ms.max(0), label: label.into(), created_at: created }))
    })
}

pub fn rename_bookmark(db_path: &Path, id: &str, label: &str) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("UPDATE bookmarks SET label = ?1 WHERE id = ?2", rusqlite::params![label, id])?;
        Ok(())
    })
}

pub fn remove_bookmark(db_path: &Path, id: &str) -> Result<()> {
    with_db(db_path, |conn| {
        initialize_schema(conn)?;
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kx-progress-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("lib.sqlite")
    }

    /// 读写往返一条龙：书签增改查删的形状与排序；进度写了能读回来。
    #[test]
    fn progress_and_bookmarks_roundtrip() {
        let db = temp_db("roundtrip");
        set_progress(&db, "t1", 12_345, Some(true), Some(1.25)).unwrap();
        mark_completed(&db, "t2", true).unwrap();

        let all = get_all_progress(&db).unwrap();
        assert_eq!(all.len(), 2);
        let t1 = all.iter().find(|p| p.track_id == "t1").unwrap();
        assert_eq!(t1.position_ms, 12_345);
        assert!(t1.completed);
        assert_eq!(t1.last_speed, Some(1.25));

        let bm = add_bookmark(&db, "t1", 5000, "段落").unwrap().unwrap();
        assert_eq!(bm.label, "段落");
        rename_bookmark(&db, &bm.id, "副歌").unwrap();
        let listed = list_bookmarks(&db, "t1").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label, "副歌");
        remove_bookmark(&db, &bm.id).unwrap();
        assert!(list_bookmarks(&db, "t1").unwrap().is_empty());
        clear_progress(&db, "t1").unwrap();
        assert_eq!(get_all_progress(&db).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    /// P1-6 的核心断言：写失败必须**上抛**，不能再被吞成「看起来成功」——
    /// 库路径指向一个建不出来的目录（父目录不存在），Connection 都开不出来。
    #[test]
    fn write_failures_propagate_instead_of_looking_successful() {
        let missing = std::env::temp_dir()
            .join(format!("kx-progress-missing-{}", std::process::id()))
            .join("no-such-dir")
            .join("lib.sqlite");
        let _ = std::fs::remove_dir_all(missing.parent().unwrap().parent().unwrap());
        assert!(set_progress(&missing, "t1", 1, None, None).is_err(), "写失败要返回 Err");
        assert!(add_bookmark(&missing, "t1", 1, "x").is_err(), "书签写失败要返回 Err");
        // 读侧区分两种空：库不存在 = Ok(空)，不是 Err（那是「还没有」的正常状态）
        assert!(list_bookmarks(&missing, "t1").unwrap().is_empty());
        assert!(get_all_progress(&missing).unwrap().is_empty());
    }
}
