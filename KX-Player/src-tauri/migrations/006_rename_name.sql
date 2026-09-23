-- 006 改名译名缓存（04 §7.6.2 的内容寻址缓存 + §7.7 的可见记录）。
-- 键 = blake3(提示词版本 + 片段)：同一原文永远同一译文，换模型不作废，改提示词措辞（涨 PROMPT_VERSION）才整体作废。
-- 只写**过完九道后置校验**的建议（notes 记的就是校验改了什么），所以 rename_apply 敢直接按它落盘。
-- created_at 而非 last_used：这张表不淘汰 —— 一行几十字节，几万条也不过几 MB，重烧 token 的代价远高于此。
CREATE TABLE IF NOT EXISTS rename_name_cache (
  cache_key  TEXT PRIMARY KEY,
  src        TEXT NOT NULL,
  dst        TEXT NOT NULL,
  notes      TEXT,
  model      TEXT,
  created_at INTEGER NOT NULL
);
