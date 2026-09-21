-- 003 分级分类树与标签（02 §5.1）。
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
