//! SQLite 连接管理 + 建表（与 Electron 版 schema 逐列一致，老库直接打开）。

pub mod library;
pub mod progress;

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

static DB_LOCK: Mutex<()> = Mutex::new(());

fn open_raw(db_path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// 串行化访问（扫描是批量写入，避免并发写冲突；读取也走同一把锁，量级无压力）
pub fn with_db<T>(db_path: &Path, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let _g = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let conn = open_raw(db_path)?;
    f(&conn)
}

pub fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS library_meta (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS artists (
          artist_id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          root_path TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_artists_name ON artists(name);
        CREATE TABLE IF NOT EXISTS albums (
          album_id INTEGER PRIMARY KEY AUTOINCREMENT,
          artist_id INTEGER NOT NULL,
          name TEXT NOT NULL,
          artist_name TEXT NOT NULL,
          cover_path TEXT,
          cover_data TEXT,
          FOREIGN KEY (artist_id) REFERENCES artists(artist_id)
        );
        CREATE INDEX IF NOT EXISTS idx_albums_artist_id ON albums(artist_id);
        CREATE TABLE IF NOT EXISTS tracks (
          id TEXT PRIMARY KEY,
          artist_id INTEGER NOT NULL,
          album_id INTEGER NOT NULL,
          name TEXT NOT NULL,
          path TEXT NOT NULL,
          duration INTEGER NOT NULL,
          artist TEXT NOT NULL,
          album TEXT NOT NULL,
          format TEXT NOT NULL,
          is_video INTEGER NOT NULL,
          cover_path TEXT,
          cover_data TEXT,
          lyrics_path TEXT,
          file_mtime REAL NOT NULL,
          file_size INTEGER NOT NULL,
          meta_title TEXT,
          meta_artist TEXT,
          genre TEXT,
          bitrate INTEGER,
          sample_rate INTEGER,
          album_cover_data TEXT,
          FOREIGN KEY (artist_id) REFERENCES artists(artist_id),
          FOREIGN KEY (album_id) REFERENCES albums(album_id)
        );
        CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks(album_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_artist_id ON tracks(artist_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);
        CREATE TABLE IF NOT EXISTS folder_nodes (
          path TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          parent_path TEXT,
          track_count INTEGER NOT NULL,
          cover_data TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_folder_nodes_parent_path ON folder_nodes(parent_path);
        CREATE TABLE IF NOT EXISTS folder_tracks (
          folder_path TEXT NOT NULL,
          track_id TEXT NOT NULL,
          PRIMARY KEY (folder_path, track_id),
          FOREIGN KEY (folder_path) REFERENCES folder_nodes(path),
          FOREIGN KEY (track_id) REFERENCES tracks(id)
        );
        CREATE INDEX IF NOT EXISTS idx_folder_tracks_track_id ON folder_tracks(track_id);

        -- 分类与标签（02-architecture §5.1 DDL）
        CREATE TABLE IF NOT EXISTS categories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          parent_id INTEGER REFERENCES categories(id) ON DELETE CASCADE,
          name TEXT NOT NULL,
          icon TEXT,
          color TEXT,
          sort_index INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS ix_categories_parent ON categories(parent_id, sort_index);
        CREATE TABLE IF NOT EXISTS category_items (
          category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
          track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          added_at INTEGER NOT NULL,
          PRIMARY KEY (category_id, track_id)
        );
        CREATE INDEX IF NOT EXISTS ix_category_items_track ON category_items(track_id);
        CREATE TABLE IF NOT EXISTS tags (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL UNIQUE,
          color TEXT,
          kind TEXT NOT NULL DEFAULT 'topic',
          use_count INTEGER NOT NULL DEFAULT 0,
          auto_generated INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS track_tags (
          track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
          source TEXT NOT NULL DEFAULT 'manual',
          PRIMARY KEY (track_id, tag_id)
        );
        CREATE INDEX IF NOT EXISTS ix_track_tags_tag ON track_tags(tag_id);
        "#,
    )?;

    let mut columns = conn.prepare("PRAGMA table_info(tracks)")?;
    let has_loudness = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "loudness_lufs");
    if !has_loudness {
        conn.execute("ALTER TABLE tracks ADD COLUMN loudness_lufs REAL", [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::initialize_schema;
    use rusqlite::Connection;

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
}
