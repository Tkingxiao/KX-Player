-- 005 AI 翻译豁免账与内容缓存（AI-15「已处理项默认跳过」）。
-- ai_translate_cache：已翻译过什么（字幕文件 / 文件夹名）。扫描时命中且「源未变 + 译文还在」→ 默认跳过。
--   判定只看 source_path + file_mtime + file_size + out_path，content_hash 只是留档不参与判定。
-- ai_subtitle_content：字幕内容的复用缓存。键 = blake3(提示词版本 + 整份原文)（ai_cache::hash），
--   相同内容的两个文件共一条；换模型不作废这条缓存，改提示词措辞才作废。有容量上限（LRU 100）。
-- 全部 IF NOT EXISTS：老库无 schema_migrations 时重放一遍即空操作（db/migrations.rs 的接管前提）。
CREATE TABLE IF NOT EXISTS ai_translate_cache (
  kind         TEXT NOT NULL,
  source_path  TEXT PRIMARY KEY,
  content_hash TEXT,
  file_mtime   REAL,
  file_size    INTEGER,
  out_path     TEXT,
  audio_path   TEXT,
  result_text  TEXT,
  translated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_ai_translate_kind ON ai_translate_cache(kind);

CREATE TABLE IF NOT EXISTS ai_subtitle_content (
  content_hash TEXT PRIMARY KEY,
  content      TEXT NOT NULL,
  size         INTEGER NOT NULL,
  last_used    INTEGER NOT NULL
);