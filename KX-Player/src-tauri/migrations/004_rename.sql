-- 004 改名批次账与已拒候选（04 §5.1:280-304）。
-- 全部 IF NOT EXISTS：老库无 schema_migrations 时重放一遍即空操作（db/migrations.rs 的接管前提）。
CREATE TABLE IF NOT EXISTS rename_batches (
  id          TEXT PRIMARY KEY,
  provider_id TEXT, model TEXT,
  created_at  INTEGER NOT NULL,
  applied_at  INTEGER,
  item_count  INTEGER NOT NULL DEFAULT 0,
  rolled_back INTEGER NOT NULL DEFAULT 0,
  note        TEXT
);
CREATE TABLE IF NOT EXISTS rename_items (
  batch_id  TEXT NOT NULL REFERENCES rename_batches(id) ON DELETE CASCADE,
  old_path  TEXT NOT NULL,
  new_path  TEXT NOT NULL,
  level     TEXT NOT NULL,
  applied   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (batch_id, old_path)
);
CREATE INDEX IF NOT EXISTS ix_rename_items_batch ON rename_items(batch_id, applied);
CREATE TABLE IF NOT EXISTS rejected_suggestions (
  target_path TEXT NOT NULL,
  suggestion  TEXT NOT NULL,
  rejected_at INTEGER NOT NULL,
  PRIMARY KEY (target_path, suggestion)
);
