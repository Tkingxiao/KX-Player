-- 007 AI 端点明文放行名单（P0-3 的显式例外）。
-- 用户在界面上逐条确认过的「非本机、非 https:443」端点（自建反代是主场景）。
-- 键是 host+port（与 scheme 无关：放行的是「这个端点可以用非标连接」，http 明文
-- 与 https 自定义端口一并覆盖）；confirmed_at 只留档。默认策略不变：没登记的
-- 公网端点照旧拒绝明文与非 443 端口。
CREATE TABLE IF NOT EXISTS ai_plaintext_allow (
  host         TEXT NOT NULL,
  port         INTEGER NOT NULL,
  confirmed_at INTEGER NOT NULL,
  PRIMARY KEY (host, port)
);
