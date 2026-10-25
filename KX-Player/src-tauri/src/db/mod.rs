//! SQLite 连接管理 + 建表入口。DDL 不在这里，见 [`migrations`] 与 `migrations/*.sql`。

pub mod library;
pub mod migrations;
pub mod progress;

use rusqlite::{Connection, OptionalExtension};
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
///
/// 跨三处（曲库事务 + 封面文件 + 设置），所以**动手之前先落意图**（P1-11）：事务提交那一刻
/// `legacy_id_map` 就再也问不出旧 ID 了，崩在提交之后的下一次启动只会看到「没有要迁的」。
pub fn migrate_legacy_ids(db_path: &Path, settings_path: &Path) -> usize {
    migrate_legacy_ids_in(db_path, settings_path, &crate::journal::dir())
}

/// 意图文件放哪儿由调用方给（生产的那个入口在 [`migrate_legacy_ids`]，测试往临时目录里摆）。
fn migrate_legacy_ids_in(db_path: &Path, settings_path: &Path, jdir: &Path) -> usize {
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
    // 封面要动哪些先问清楚（只读）。放在事务之前是为了能写进意图 —— 事务之后再问，
    // `tracks.id` 已经是新的，旧 ID 一条也查不出来。
    let covers = match with_db(db_path, |conn| cover_rewrites(conn, &map)) {
        Ok(c) => c,
        Err(e) => {
            crate::paths::append_log(&format!("id 迁移中止：读取封面路径失败 {e}"));
            return 0;
        }
    };
    let intent = LegacyIntent {
        map: map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        covers: covers
            .iter()
            .filter(|c| c.present)
            .map(|c| (c.from.to_string_lossy().into_owned(), c.to.to_string_lossy().into_owned()))
            .collect(),
    };
    if let Err(e) = write_legacy_intent(jdir, &intent) {
        // 写不下意图就不迁：这三处没有共同的事务，没有线索的迁移崩了一半只能靠人猜
        crate::paths::append_log(&format!("id 迁移中止（数据未改动）：{e}"));
        return 0;
    }
    checkpoint(db_path);
    backup_once(db_path);
    backup_once(settings_path);

    if let Err(e) = with_db(db_path, |conn| {
        // 改写期间必须关掉外键：子表先指向新 ID 时，tracks 的主键还是旧 ID。
        // 而 `PRAGMA foreign_keys` 在事务内是空操作，只能放在开启事务之前。
        conn.execute_batch("PRAGMA foreign_keys=OFF")?;
        let applied = apply_id_map(conn, &map, &covers);
        conn.execute_batch("PRAGMA foreign_keys=ON")?;
        applied
    }) {
        crate::paths::append_log(&format!("id 迁移中止（数据未改动，备份见 *.pre-uuid5）：{e}"));
        return 0;
    }
    for c in covers.iter().filter(|c| c.present) {
        // 不覆盖：新名字上已经有文件（多半是重扫又生成了一张）就跳过，留给人看
        if let Err(e) = crate::rename::paths::move_one(&c.from.to_string_lossy(), &c.to.to_string_lossy()) {
            crate::paths::append_log(&format!("封面改名失败：{}（{e}）", c.from.display()));
        }
    }
    remap_settings_ids(settings_path, &map);
    remove_legacy_intent(jdir);
    map.len()
}

/// 旧 ID 迁移的意图文件（P1-11）。事务提交之后崩掉的话，`legacy_id_map` 再也问不出旧 ID，
/// 封面永远搬不过去、设置永远指不到新条目 —— 这份文件就是那条唯一的线索。
const LEGACY_INTENT: &str = "legacy-ids.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyIntent {
    /// 旧 ID → 新 ID：settings 里以条目 ID 寻址的三处按它改写
    map: HashMap<String, String>,
    /// 要跟着改名的封面（旧路径, 新路径）；只收此刻真的在盘上的
    covers: Vec<(String, String)>,
}

fn write_legacy_intent(jdir: &Path, intent: &LegacyIntent) -> Result<(), String> {
    let body = serde_json::to_string_pretty(intent).map_err(|e| format!("序列化失败：{e}"))?;
    crate::journal::put(jdir, LEGACY_INTENT, &body)
}

fn remove_legacy_intent(jdir: &Path) {
    crate::journal::remove(jdir, LEGACY_INTENT);
}

/// 上一次旧 ID 迁移没做完的补偿（P1-11）。`None` = 没有未完成的迁移。
pub fn recover_legacy_ids(settings_path: &Path) -> Option<String> {
    recover_legacy_ids_in(&crate::journal::dir(), settings_path)
}

/// 与改名的恢复不同，这一步**会动盘**：封面是应用自己管的文件（名字就是条目 ID），
/// 用户不会去改它，而放着不搬的后果是「封面全没了」。判据只有「旧名在、新名不在」这一条，
/// 新名已经在就绝不覆盖。两件活都是幂等的（搬过了就不再搬、值已经是新 ID 不算改动），
/// 所以跑几遍和跑一遍一样。
fn recover_legacy_ids_in(jdir: &Path, settings_path: &Path) -> Option<String> {
    let path = jdir.join(LEGACY_INTENT);
    if !path.is_file() {
        return None;
    }
    let intent: LegacyIntent = match crate::journal::read(&path)
        .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("解析失败：{e}")))
    {
        Ok(v) => v,
        Err(e) => return Some(format!("意图文件读不出来（{e}），未做任何改动")),
    };

    let (mut moved, mut nothing, mut retry) = (0usize, 0usize, Vec::new());
    let mut conflicts = Vec::new();
    for (old, new) in &intent.covers {
        match (Path::new(old).is_file(), Path::new(new).exists()) {
            (true, false) => match crate::rename::paths::move_one(old, new) {
                Ok(()) => moved += 1,
                Err(e) => retry.push(format!("{old}：{e}")),
            },
            (false, true) => nothing += 1, // 上一次恢复到一半又崩：这条已经搬过了
            (true, true) => conflicts.push(format!("{old}：新名 {new} 已被占用，未覆盖")),
            (false, false) => nothing += 1,
        }
    }
    let n = intent.map.len();
    remap_settings_ids(settings_path, &intent.map);
    let mut line = format!(
        "封面补搬 {moved} 个 · 无需搬 {nothing} 个 · 说不清 {} 个 · 设置按 {n} 条映射重写",
        conflicts.len()
    );
    if !retry.is_empty() {
        // 搬不动是暂时性的（占用/权限），意图留着下次启动再试
        line.push_str(&format!("；{} 个没搬成：{}", retry.len(), retry.join(" | ")));
    }
    if !conflicts.is_empty() {
        line.push_str(&format!("；{}", conflicts.join(" | ")));
    }
    if retry.is_empty() {
        remove_legacy_intent(jdir);
    }
    Some(line)
}

/// 一条要跟着改名的封面。`present` 说的是文件此刻在不在盘上 —— 不在也照样要改 `cover_path`
/// （库里记的路径跟着新 ID 走），只是没有文件可搬。
struct CoverRewrite {
    id: String,
    from: std::path::PathBuf,
    to: std::path::PathBuf,
    present: bool,
}

/// 这一次迁移要动哪些封面文件（只读）。判据与老实现一字不差：`cover_path` 的文件名
/// 必须正好是 `{旧 ID}.jpg`，别的封面（用户自己挑的图）不碰。
fn cover_rewrites(conn: &Connection, map: &HashMap<String, String>) -> rusqlite::Result<Vec<CoverRewrite>> {
    let mut q = conn.prepare("SELECT cover_path FROM tracks WHERE id = ?1")?;
    let mut out = Vec::new();
    for (old, new) in map {
        let cover = q.query_row([old], |r| r.get::<_, Option<String>>(0)).optional()?.flatten();
        let Some(cover) = cover else { continue };
        let file = Path::new(&cover);
        if file.file_name().map(|n| n.to_string_lossy().into_owned()) != Some(format!("{old}.jpg")) {
            continue;
        }
        out.push(CoverRewrite {
            id: old.clone(),
            from: file.to_path_buf(),
            to: file.with_file_name(format!("{new}.jpg")),
            present: file.is_file(),
        });
    }
    Ok(out)
}

/// 在一个事务里改 5 张子表 → 封面路径 → tracks 主键（外键已在调用方关闭）。
/// 封面文件的改名不在这里：事务提交之后才动磁盘（`covers` 由 [`cover_rewrites`] 在事务之前备好）。
fn apply_id_map(conn: &Connection, map: &HashMap<String, String>, covers: &[CoverRewrite]) -> rusqlite::Result<()> {
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

    {
        let mut upd = tx.prepare("UPDATE tracks SET cover_path = ?2 WHERE id = ?1")?;
        for c in covers {
            upd.execute((&c.id, c.to.to_string_lossy().into_owned()))?;
        }
    }
    tx.execute(
        "UPDATE tracks SET id = (SELECT new_id FROM legacy_map WHERE legacy_map.old_id = tracks.id) \
         WHERE id IN (SELECT old_id FROM legacy_map)",
        [],
    )?;
    tx.execute("DROP TABLE temp.legacy_map", [])?;
    tx.commit()?;
    Ok(())
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
    use super::{
        initialize_schema, migrations, migrate_legacy_ids_in, open_raw, recover_legacy_ids_in, with_db,
        LEGACY_INTENT,
    };
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
        let jdir = dir.join("journal");
        std::fs::create_dir_all(&covers).expect("封面目录");
        std::fs::create_dir_all(&jdir).expect("意图目录");
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
        crate::db::progress::set_progress(&db, legacy, 4321, Some(false), None).expect("写进度");
        crate::db::progress::add_bookmark(&db, legacy, 1000, "标记").expect("写书签");
        std::fs::write(
            &settings,
            format!(r#"{{"recents":["{legacy}"],"favs":[{{"id":"fav-default","trackIds":["{legacy}"]}}],"pls":[{{"id":"l1","trackIds":["{legacy}","ghost"]}}],"vol":0.5}}"#),
        )
        .expect("写入设置");

        assert_eq!(migrate_legacy_ids_in(&db, &settings, &jdir), 1);

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
        assert!(
            crate::journal::list(&jdir, LEGACY_INTENT).is_empty(),
            "跑完了就要把意图收掉，不然下次启动会误以为迁移没做完"
        );

        assert_eq!(migrate_legacy_ids_in(&db, &settings, &jdir), 0, "再次执行应为空操作");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 崩溃窗口（P1-11）：事务提交之后、封面与设置还没改完。那一刻 `legacy_id_map` 已经是空的，
    /// 没有意图文件的话这一步永远不会有人来补 —— 封面和设置会一直指着已经不存在的老 ID。
    /// 现场是**手工摆回**的：先正经跑一遍迁移，再把封面文件与设置退回崩溃那一刻的样子。
    #[test]
    fn a_crashed_legacy_migration_is_finished_by_the_next_start() {
        let legacy = "0123456789ab";
        let dir = temp_dir("recov");
        let db = dir.join("library.db");
        let settings = dir.join("settings.json");
        let covers = dir.join("covers");
        let jdir = dir.join("journal");
        std::fs::create_dir_all(&covers).expect("封面目录");
        std::fs::create_dir_all(&jdir).expect("意图目录");
        let new = crate::scanner::meta::file_id("C:\\Music\\A\\song.mp3");
        with_db(&db, |conn| {
            initialize_schema(conn)?;
            conn.execute("INSERT INTO artists (artist_id, name, root_path) VALUES (1, 'a', 'C:\\Music')", [])?;
            conn.execute("INSERT INTO albums (album_id, artist_id, name, artist_name) VALUES (1, 1, 'al', 'a')", [])?;
            conn.execute(
                "INSERT INTO tracks (id, artist_id, album_id, name, path, duration, artist, album, \
                 format, is_video, cover_path, file_mtime, file_size) \
                 VALUES (?1, 1, 1, 's', 'C:\\Music\\A\\song.mp3', 100, 'a', 'al', 'mp3', 0, ?2, 0, 1)",
                (legacy, covers.join(format!("{legacy}.jpg")).to_str().unwrap()),
            )?;
            Ok(())
        })
        .expect("装配旧库");
        std::fs::write(covers.join(format!("{legacy}.jpg")), b"jpeg").expect("旧封面");
        let legacy_settings = format!(r#"{{"recents":["{legacy}"],"vol":0.5}}"#);
        std::fs::write(&settings, &legacy_settings).expect("写入设置");

        assert_eq!(migrate_legacy_ids_in(&db, &settings, &jdir), 1);
        // 退回崩溃那一刻：曲库已经迁完、封面还在旧名下、设置还指着旧 ID、意图文件还在
        std::fs::rename(covers.join(format!("{new}.jpg")), covers.join(format!("{legacy}.jpg"))).expect("退回封面");
        std::fs::write(&settings, &legacy_settings).expect("退回设置");
        let body = serde_json::json!({
            "map": { legacy: new },
            "covers": [[covers.join(format!("{legacy}.jpg")).to_string_lossy(), covers.join(format!("{new}.jpg")).to_string_lossy()]],
        })
        .to_string();
        crate::atomic::write_str(&jdir.join(LEGACY_INTENT), &body).expect("摆意图文件");

        let line = recover_legacy_ids_in(&jdir, &settings).expect("有意图文件就该有恢复");
        assert!(line.contains("封面补搬 1 个"), "{line}");
        assert!(covers.join(format!("{new}.jpg")).is_file(), "封面要补搬过去");
        assert!(!covers.join(format!("{legacy}.jpg")).exists());
        let doc = crate::settingsio::load(&settings);
        assert_eq!(doc["recents"][0].as_str(), Some(new.as_str()), "设置要按意图里的映射重写");
        assert_eq!(doc["vol"].as_f64(), Some(0.5), "无关设置键不该动");
        assert!(crate::journal::list(&jdir, LEGACY_INTENT).is_empty(), "补完了就收掉意图");

        // 幂等：再跑一遍不该有任何新动作（这时已经没有意图文件了）
        assert!(recover_legacy_ids_in(&jdir, &settings).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 新名字上已经有封面（多半是重扫又生成了一张）：**绝不覆盖**，报出来给人看，
    /// 但设置那半照样补完 —— 剩下的只是一个没人用的旧文件，不值得一直挂着一份未完成的意图。
    #[test]
    fn the_legacy_recovery_never_overwrites_a_newer_cover() {
        let dir = temp_dir("coverclash");
        let settings = dir.join("settings.json");
        let jdir = dir.join("journal");
        std::fs::create_dir_all(&jdir).expect("意图目录");
        let (old, new) = (dir.join("旧.jpg"), dir.join("新.jpg"));
        std::fs::write(&old, "老的").unwrap();
        std::fs::write(&new, "新的").unwrap();
        std::fs::write(&settings, r#"{"recents":["0123456789ab"]}"#).unwrap();
        let body = serde_json::json!({
            "map": { "0123456789ab": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
            "covers": [[old.to_string_lossy(), new.to_string_lossy()]],
        })
        .to_string();
        crate::atomic::write_str(&jdir.join(LEGACY_INTENT), &body).expect("摆意图文件");

        let line = recover_legacy_ids_in(&jdir, &settings).expect("有意图文件就该有恢复");
        assert!(line.contains("已被占用"), "{line}");
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "新的", "新封面不许被覆盖");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "老的", "旧文件也不动");
        assert_eq!(
            crate::settingsio::load(&settings)["recents"][0].as_str(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            "封面搬不动不影响设置那一半"
        );
        assert!(crate::journal::list(&jdir, LEGACY_INTENT).is_empty(), "能补的都补了，别再挂着一份意图");
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
