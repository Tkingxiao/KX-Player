-- 001 曲库主表。沿用 Electron 版列名，老库可直接 IF NOT EXISTS 接管。
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
  loudness_lufs REAL,
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

-- 播放进度与书签（≥95% 记为听完由前端判定，这里只存事实）
CREATE TABLE IF NOT EXISTS play_progress (
  track_id     TEXT PRIMARY KEY,
  position_ms  INTEGER NOT NULL DEFAULT 0,
  completed    INTEGER NOT NULL DEFAULT 0,
  play_count   INTEGER NOT NULL DEFAULT 1,
  played_at    INTEGER NOT NULL DEFAULT 0,
  last_speed   REAL
);
CREATE INDEX IF NOT EXISTS idx_progress_played_at ON play_progress(played_at DESC);
CREATE TABLE IF NOT EXISTS bookmarks (
  id         TEXT PRIMARY KEY,
  track_id   TEXT NOT NULL,
  at_ms      INTEGER NOT NULL,
  label      TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_bookmarks_track ON bookmarks(track_id, at_ms);
