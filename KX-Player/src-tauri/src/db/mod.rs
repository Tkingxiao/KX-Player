//! SQLite 连接管理 + 建表入口。DDL 不在这里，见 [`migrations`] 与 `migrations/*.sql`。

pub mod library;
pub mod migrations;
pub mod progress;

use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

static DB_LOCK: Mutex<()> = Mutex::new(());

fn open_raw(db_path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // 02 §7.2 要求 IMMEDIATE：`unchecked_transaction()` 读这个字段决定 BEGIN 的写法。
    // 在连接层设一次，三处写事务全覆盖，DEFERRED 那种「读到一半别人已经占住写锁」的窗口就不存在了。
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    Ok(conn)
}

/// 串行化访问（扫描是批量写入，避免并发写冲突；读取也走同一把锁，量级无压力）
pub fn with_db<T>(db_path: &Path, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let _g = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let conn = open_raw(db_path)?;
    f(&conn)
}

/// 建表入口：把连接拉到最新迁移版本。幂等，每次打开连接都可以调。
pub fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
    migrations::apply(conn)?;
    Ok(())
}

/// 同 [`with_db`]，但闭包返回自己的错误文案。改名主干的失败原因是给用户看的中文句子
/// （「记账失败（…），已撤销」这类），套进 `rusqlite::Error` 再拆出来只会绕一圈。
/// 建表在这里做一次：调用方不用记着调，忘了也不会得到一句「no such table: rename_batches」。
pub fn with_db_text<T>(db_path: &Path, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let _g = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let conn = open_raw(db_path).map_err(|e| format!("打开曲库失败：{e}"))?;
    initialize_schema(&conn).map_err(|e| format!("建表失败：{e}"))?;
    f(&conn)
}

/// P0-64 启动自检：校验曲库完整性。损坏只降级提示、不阻止启动 ——
/// 曲库仍可通过重新扫描重建，把用户挡在门外不是更坏的结局。
/// 同时充当迁移失败的兜底：`initialize_schema` 的错误一路上抛会被调用方吞掉、
/// 表现为「曲库凭空变空」，这里把它显式写进日志并提示用户。
pub fn check_integrity() {
    let db_path = crate::paths::library_db_path();
    if !db_path.exists() {
        return;
    }
    let report = match with_db(&db_path, initialize_schema)
        .and_then(|()| with_db(&db_path, migrations::integrity_check))
    {
        Ok(status) if status == "ok" => return,
        Ok(status) => status,
        Err(e) => format!("无法打开或建表：{e}"),
    };
    crate::paths::append_log(&format!("启动自检：integrity_check 未通过 → {report}"));
    crate::state::startup_warn(
        "曲库数据库完整性检查未通过，播放进度与封面可能不完整。若持续异常，请在设置中删除 library.db 后重新扫描。",
    );
}

/// 以条目 ID 寻址、且**不**被扫描重建的表 —— 换 ID 方案时必须跟着改写。
/// 表名/列名是编译期常量，不进用户输入，拼进 SQL 无注入面。
const TRACK_ID_TABLES: &[(&str, &str)] = &[
    ("folder_tracks", "track_id"),
    ("category_items", "track_id"),
    ("track_tags", "track_id"),
    ("play_progress", "track_id"),
    ("bookmarks", "track_id"),
];

fn legacy_id_map(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    if !table_exists(conn, "tracks")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT id, path FROM tracks")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, path) = row?;
        let new_id = crate::scanner::meta::file_id(&path);
        if new_id != id {
            out.push((id, new_id));
        }
    }
    Ok(out)
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?;
    let mut rows = stmt.query([name])?;
    Ok(rows.next()?.is_some())
}

fn checkpoint(db_path: &Path) {
    let _ = with_db(db_path, |conn| {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    });
}

/// 就地复制一份 `.pre-uuid5`；已存在则不覆盖（第二次迁移不能拿坏数据冲掉好备份）。
fn backup_once(path: &Path) {
    if !path.is_file() {
        return;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return };
    let dst = path.with_file_name(format!("{name}.pre-uuid5"));
    if !dst.exists() {
        let _ = std::fs::copy(path, &dst);
    }
}

/// 旧版条目 ID（`md5(反斜杠路径)` 前 12 位）→ UUIDv5 的全量改写。
///
/// 判据是纯函数式的（只改 `id != uuid5(path)` 的行），所以每次启动都可以调用，
/// 迁完第二次即空操作，无需版本标记。封面文件名与 settings 里的
/// `recents/favs/pls` 一起改，否则进度、书签、收藏与缓存会集体失联。
pub fn migrate_legacy_ids(db_path: &Path, settings_path: &Path) -> usize {
    if !db_path.exists() {
        return 0;
    }
    let map: HashMap<String, String> = match with_db(db_path, legacy_id_map) {
        Ok(m) if !m.is_empty() => m.into_iter().collect(),
        Ok(_) => return 0,
        Err(e) => {
            crate::paths::append_log(&format!("id 迁移中止：读取 tracks 失败 {e}"));
            return 0;
        }
    };
    checkpoint(db_path);
    backup_once(db_path);
    backup_once(settings_path);

    let renames = match with_db(db_path, |conn| {
        // 改写期间必须关掉外键：子表先指向新 ID 时，tracks 的主键还是旧 ID。
        // 而 `PRAGMA foreign_keys` 在事务内是空操作，只能放在开启事务之前。
        conn.execute_batch("PRAGMA foreign_keys=OFF")?;
        let applied = apply_id_map(conn, &map);
        conn.execute_batch("PRAGMA foreign_keys=ON")?;
        applied
    }) {
        Ok(r) => r,
        Err(e) => {
            crate::paths::append_log(&format!("id 迁移中止（数据未改动，备份见 *.pre-uuid5）：{e}"));
            return 0;
        }
    };
    for (old, new) in renames {
        let _ = std::fs::rename(&old, &new);
    }
    remap_settings_ids(settings_path, &map);
    map.len()
}

/// 在一个事务里改 5 张子表 → 封面路径 → tracks 主键（外键已在调用方关闭）。
/// 返回需要改名的封面文件对，交给调用方在事务提交后再动磁盘。
fn apply_id_map(conn: &Connection, map: &HashMap<String, String>) -> rusqlite::Result<Vec<(std::path::PathBuf, std::path::PathBuf)>> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("CREATE TEMP TABLE legacy_map (old_id TEXT PRIMARY KEY, new_id TEXT NOT NULL)", [])?;
    {
        let mut ins = tx.prepare("INSERT INTO legacy_map (old_id, new_id) VALUES (?1, ?2)")?;
        for (old, new) in map {
            ins.execute((old, new))?;
        }
    }
    for (table, column) in TRACK_ID_TABLES {
        if !table_exists(&tx, table)? {
            continue;
        }
        tx.execute(
            &format!(
                "UPDATE {table} SET {column} = \
                 (SELECT new_id FROM legacy_map WHERE legacy_map.old_id = {table}.{column}) \
                 WHERE {column} IN (SELECT old_id FROM legacy_map)"
            ),
            [],
        )?;
    }

    let mut renames = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, cover_path FROM tracks \
             WHERE cover_path IS NOT NULL AND id IN (SELECT old_id FROM legacy_map)",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let mut upd = tx.prepare("UPDATE tracks SET cover_path = ?2 WHERE id = ?1")?;
        for (id, cover_path) in rows {
            let Some(new_id) = map.get(&id) else {
                continue;
            };
            let file = std::path::Path::new(&cover_path);
            if file.file_name().map(|n| n.to_string_lossy().into_owned()) != Some(format!("{id}.jpg")) {
                continue;
            }
            let new_file = file.with_file_name(format!("{new_id}.jpg"));
            if file.is_file() {
                renames.push((file.to_path_buf(), new_file.clone()));
            }
            upd.execute((id, new_file.to_string_lossy().into_owned()))?;
        }
    }
    tx.execute(
        "UPDATE tracks SET id = (SELECT new_id FROM legacy_map WHERE legacy_map.old_id = tracks.id) \
         WHERE id IN (SELECT old_id FROM legacy_map)",
        [],
    )?;
    tx.execute("DROP TABLE temp.legacy_map", [])?;
    tx.commit()?;
    Ok(renames)
}

/// settings.json 中以条目 ID 寻址的三处：`recents[*]`、`favs[*].trackIds[*]`、`pls[*].trackIds[*]`。
fn remap_settings_ids(settings_path: &Path, lookup: &HashMap<String, String>) {
    if !settings_path.is_file() {
        return;
    }
    let mut doc = crate::settingsio::load(settings_path);
    let mut changed = false;
    let remap_array = |arr: &mut Vec<serde_json::Value>, changed: &mut bool| {
        for item in arr.iter_mut() {
            if let Some(new) = item.as_str().and_then(|s| lookup.get(s)) {
                *item = serde_json::Value::String(new.clone());
                *changed = true;
            }
        }
    };
    if let Some(root) = doc.as_object_mut() {
        if let Some(recents) = root.get_mut("recents").and_then(|v| v.as_array_mut()) {
            remap_array(recents, &mut changed);
        }
        for key in ["favs", "pls"] {
            let Some(lists) = root.get_mut(key).and_then(|v| v.as_array_mut()) else {
                continue;
            };
            for list in lists.iter_mut() {
                if let Some(ids) = list.get_mut("trackIds").and_then(|v| v.as_array_mut()) {
                    remap_array(ids, &mut changed);
                }
            }
        }
    }
    if changed {
        crate::settingsio::save(settings_path, &doc);
    }
}

#[cfg(test)]
mod tests {
    use super::{initialize_schema, migrations, migrate_legacy_ids, open_raw, with_db};
    use rusqlite::Connection;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("kx-id-mig-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).expect("查询计数")
    }

    #[test]
    fn schema_adds_loudness_column_idempotently() {
        let conn = Connection::open_in_memory().expect("open sqlite");
        initialize_schema(&conn).expect("first schema init");
        initialize_schema(&conn).expect("second schema init");

        let mut stmt = conn.prepare("PRAGMA table_info(tracks)").expect("table info");
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get(1))
            .expect("query columns")
            .collect::<Result<_, _>>()
            .expect("collect columns");
        assert!(columns.iter().any(|name| name == "loudness_lufs"));
    }

    /// `02 §7.2` 要求写事务是 IMMEDIATE：`BEGIN` 一执行就拿到写锁。
    /// 判据用「事务里还没写任何东西，第二条连接就该撞 BUSY」来区分 ——
    /// DEFERRED 下这条 UPDATE 会静默成功。
    #[test]
    fn write_transactions_take_the_write_lock_at_begin() {
        let dir = temp_dir("immediate");
        let db = dir.join("library.db");
        with_db(&db, |conn| {
            initialize_schema(conn)?;
            conn.execute("INSERT INTO library_meta (key, value) VALUES ('fileCount', '1')", [])?;
            Ok(())
        })
        .expect("建库");

        let holder = open_raw(&db).expect("连接 a");
        let tx = holder.unchecked_transaction().expect("BEGIN");
        let other = open_raw(&db).expect("连接 b");
        let err = other
            .execute("UPDATE library_meta SET value = '2' WHERE key = 'fileCount'", [])
            .expect_err("写锁已被持有，第二条连接不该写得动");
        assert_eq!(
            err.sqlite_error_code(),
            Some(rusqlite::ffi::ErrorCode::DatabaseBusy),
            "期望 SQLITE_BUSY，实际 {err:?}"
        );
        drop(tx);
    }

    /// 换 ID 方案时最容易丢的就是「不随扫描重建」的那些引用：进度、书签、分类、
    /// 标签、文件夹关联、封面文件名和 settings 里的收藏列表。逐张盯住。
    #[test]
    fn migrate_legacy_ids_rewrites_every_id_bearer() {
        let legacy = "0123456789ab";
        let track_path = "C:\\Music\\Album\\song.mp3";
        let new = crate::scanner::meta::file_id(track_path);
        assert_eq!(new.len(), 32, "UUIDv5 应为 32 位十六进制");
        assert_ne!(new, legacy);

        let dir = temp_dir("ids");
        let db = dir.join("library.db");
        let settings = dir.join("settings.json");
        let covers = dir.join("covers");
        std::fs::create_dir_all(&covers).expect("封面目录");
        let old_cover = covers.join(format!("{legacy}.jpg"));
        std::fs::write(&old_cover, b"jpeg").expect("写入旧封面");

        with_db(&db, |conn| {
            initialize_schema(conn)?;
            conn.execute(
                "INSERT INTO artists (artist_id, name, root_path) VALUES (1, 'a', 'C:\\Music')",
                [],
            )?;
            conn.execute(
                "INSERT INTO albums (album_id, artist_id, name, artist_name) VALUES (1, 1, 'al', 'a')",
                [],
            )?;
            conn.execute(
                "INSERT INTO tracks (id, artist_id, album_id, name, path, duration, artist, album, \
                 format, is_video, cover_path, file_mtime, file_size) \
                 VALUES (?1, 1, 1, 's', ?2, 100, 'a', 'al', 'mp3', 0, ?3, 0, 1)",
                (legacy, track_path, old_cover.to_str().unwrap()),
            )?;
            conn.execute(
                "INSERT INTO folder_nodes (path, name, parent_path, track_count) VALUES ('C:\\Music', 'Music', NULL, 1)",
                [],
            )?;
            conn.execute("INSERT INTO folder_tracks (folder_path, track_id) VALUES ('C:\\Music', ?1)", [legacy])?;
            conn.execute("INSERT INTO categories (id, name, created_at) VALUES (1, 'c', 0)", [])?;
            conn.execute("INSERT INTO category_items (category_id, track_id, added_at) VALUES (1, ?1, 0)", [legacy])?;
            conn.execute("INSERT INTO tags (id, name) VALUES (1, 't')", [])?;
            conn.execute("INSERT INTO track_tags (track_id, tag_id) VALUES (?1, 1)", [legacy])?;
            Ok(())
        })
        .expect("装配旧库");
        crate::db::progress::set_progress(&db, legacy, 4321, Some(false), None);
        crate::db::progress::add_bookmark(&db, legacy, 1000, "标记");
        std::fs::write(
            &settings,
            format!(r#"{{"recents":["{legacy}"],"favs":[{{"id":"fav-default","trackIds":["{legacy}"]}}],"pls":[{{"id":"l1","trackIds":["{legacy}","ghost"]}}],"vol":0.5}}"#),
        )
        .expect("写入设置");

        assert_eq!(migrate_legacy_ids(&db, &settings), 1);

        with_db(&db, |conn| {
            assert_eq!(scalar_i64(conn, &format!("SELECT COUNT(*) FROM tracks WHERE id='{new}'")), 1);
            for table in ["folder_tracks", "category_items", "track_tags", "play_progress", "bookmarks"] {
                assert_eq!(
                    scalar_i64(conn, &format!("SELECT COUNT(*) FROM {table} WHERE track_id='{new}'")),
                    1,
                    "{table} 未跟随改写"
                );
            }
            assert_eq!(scalar_i64(conn, "SELECT COUNT(*) FROM tracks WHERE id='0123456789ab'"), 0);
            let cover: String = conn.query_row("SELECT cover_path FROM tracks WHERE id=?1", [&new], |r| r.get(0))?;
            assert!(cover.ends_with(&format!("{new}.jpg")), "cover_path 未改写：{cover}");
            Ok(())
        })
        .expect("校验改写结果");

        let doc = crate::settingsio::load(&settings);
        assert_eq!(doc["recents"][0].as_str(), Some(new.as_str()));
        assert_eq!(doc["favs"][0]["trackIds"][0].as_str(), Some(new.as_str()));
        assert_eq!(doc["pls"][0]["trackIds"][0].as_str(), Some(new.as_str()));
        assert_eq!(doc["pls"][0]["trackIds"][1].as_str(), Some("ghost"), "未知 ID 不应被动");
        assert_eq!(doc["vol"].as_f64(), Some(0.5), "无关设置键应保持原样");
        assert!(covers.join(format!("{new}.jpg")).is_file() && !old_cover.exists(), "封面文件未改名");
        assert!(db.with_extension("db.pre-uuid5").is_file(), "缺少改前备份");

        assert_eq!(migrate_legacy_ids(&db, &settings), 0, "再次执行应为空操作");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 文件库走的是 `open_raw` 的 WAL + `foreign_keys=ON`，内存库覆盖不到这一层；
    /// 盯住两件出事就致命的事：重复建表把数据洗掉、外键 pragma 与迁移事务互锁。
    #[test]
    fn migrations_replay_harmlessly_on_a_file_database() {
        let dir = temp_dir("schema");
        let db = dir.join("library.db");
        with_db(&db, |conn| {
            initialize_schema(conn)?;
            conn.execute("INSERT INTO play_progress (track_id, played_at) VALUES ('x', 1)", [])?;
            let fk_on: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            assert_eq!(fk_on, 1, "迁移后外键开关被改掉");
            Ok(())
        })
        .expect("首次建表");
        with_db(&db, |conn| {
            initialize_schema(conn)?;
            let kept = scalar_i64(conn, "SELECT COUNT(*) FROM play_progress WHERE track_id='x'");
            assert_eq!(kept, 1, "重复建表洗掉了已有进度");
            assert_eq!(migrations::integrity_check(conn)?, "ok");
            Ok(())
        })
        .expect("重放建表");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
