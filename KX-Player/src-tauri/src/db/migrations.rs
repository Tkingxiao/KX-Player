//! 版本化建表：DDL 外置在 `src-tauri/migrations/*.sql`，经 `include_str!` 编进二进制，
//! 运行时按版本号重放未应用的部分（`schema_migrations` 记账）。
//!
//! 老库（表已在、没有 `schema_migrations`）直接接管：所有语句都是
//! `CREATE … IF NOT EXISTS`，重放一遍即空操作，随后正常记账。

use rusqlite::Connection;

/// `(版本号, 文件名, SQL)`。版本号与 02 §5.1 的文件名一一对应、只增不减；
/// 改名或删号会让已应用过的库失去对齐依据。
/// `2 / 002_search_index.sql`（FTS5，条目数 >20,000 时启用）留给筛选下沉那条改动。
pub(crate) const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "001_init.sql", include_str!("../../migrations/001_init.sql")),
    (3, "003_taxonomy.sql", include_str!("../../migrations/003_taxonomy.sql")),
];

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// SQLite 的 `ALTER TABLE … ADD COLUMN` 没有 IF NOT EXISTS，只能程序化判断。
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if exists {
        return Ok(());
    }
    conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
    Ok(())
}

/// 应用全部未执行的迁移。返回本次实际应用的数量（0 表示已是最新）。
pub fn apply(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
    )?;
    let mut applied = 0usize;
    for (version, _file, sql) in MIGRATIONS {
        let done: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            [*version],
            |r| r.get(0),
        )?;
        if done > 0 {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            (*version, now_ms()),
        )?;
        tx.commit()?;
        applied += 1;
    }
    ensure_column(conn, "tracks", "loudness_lufs", "REAL")?;
    Ok(applied)
}

/// P0-64 启动自检：`PRAGMA integrity_check`。返回 "ok" 之外的内容即数据库已损坏。
pub fn integrity_check(conn: &Connection) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare("PRAGMA integrity_check")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    Ok(rows.join("; "))
}

#[cfg(test)]
mod tests {
    use super::{apply, integrity_check, MIGRATIONS};
    use rusqlite::Connection;

    fn versions(conn: &Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .expect("读取迁移版本");
        stmt.query_map([], |r| r.get(0))
            .expect("查询迁移版本")
            .collect::<Result<_, _>>()
            .expect("收集迁移版本")
    }

    #[test]
    fn apply_is_idempotent_and_records_every_version() {
        let conn = Connection::open_in_memory().expect("内存库");
        assert_eq!(apply(&conn).expect("首次应用"), MIGRATIONS.len());
        assert_eq!(apply(&conn).expect("重复应用"), 0);
        let expected: Vec<i64> = MIGRATIONS.iter().map(|(v, _, _)| *v).collect();
        assert_eq!(versions(&conn), expected);
    }

    /// 老库没有 schema_migrations，表却已经在了 —— 接管而不是报错。
    /// 列形状按真实老库给（被索引引用的列都在），少掉的那一列由 ensure_column 补。
    #[test]
    fn apply_adopts_a_preexisting_unversioned_database() {
        let conn = Connection::open_in_memory().expect("内存库");
        conn.execute_batch(
            "CREATE TABLE tracks (
               id TEXT PRIMARY KEY, album_id INTEGER NOT NULL, artist_id INTEGER NOT NULL, path TEXT NOT NULL);
             CREATE TABLE play_progress (track_id TEXT PRIMARY KEY, played_at INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("模拟老库");
        assert!(apply(&conn).is_ok());
        assert_eq!(versions(&conn).len(), MIGRATIONS.len());
        assert_eq!(integrity_check(&conn).expect("完整性检查"), "ok");

        let mut stmt = conn.prepare("PRAGMA table_info(tracks)").expect("列信息");
        let columns: Vec<String> = stmt
            .query_map([], |r| r.get(1))
            .expect("查询列")
            .collect::<Result<_, _>>()
            .expect("收集列");
        assert!(columns.iter().any(|name| name == "loudness_lufs"));
    }
}
