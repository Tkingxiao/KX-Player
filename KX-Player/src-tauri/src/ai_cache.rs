//! AI 翻译的「已翻译账」与「内容复用缓存」（AI-15「已处理项默认跳过」）。
//!
//! 为什么要有这一层：翻译一次要烧 token，重复翻译同一个文件 / 同一段字幕是纯浪费。
//! 这里把「翻译过什么」落库（`ai_translate_cache`），再次扫描时命中且源文件没变、译文
//! 还在 → 默认跳过；再把「原文内容 → 译文」复用缓存（`ai_subtitle_content`）按
//! `blake3(提示词版本 + 原文)` 去重，相同内容的多个文件共一条缓存、且有容量上限（LRU 淘汰）——「合理选择和压缩」。
//!
//! 本文件只管读库/写库的纯函数，连接由调用方（命令 / 翻译任务）经 `db::with_db_text`
//! 注入；缓存键归一化用 crate::model::normalize_path，避免正反斜杠一双键。
//! 译文文件本身（.zh.*）不进这条账也不进扫描：排除规则在 `ai_prompt::is_translated_output`。

use rusqlite::Connection;
use std::path::Path;

/// 译文文件（.zh.*）由 `ai_prompt::is_translated_output` 在扫描与管线两处排除，不会走到这条账里。
/// 音频/视频扩展名：字幕（.lrc 尤其）常跟随同名媒体文件，记录配对路径。
const MEDIA_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "aac", "ogg", "oga", "opus", "m4a", "m4b", "wma", "ape", "wv", "aiff",
    "mp4", "mkv", "mov", "webm", "avi",
];

/// 字幕内容复用缓存的上限（条）。生效手段是「超了删最旧的」。
const CONTENT_CACHE_CAP: i64 = 100;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// 内容复用缓存的键 = `blake3(提示词版本 + 原文)`（`04 §7.6.2` 一致性账）。
/// **键里没有 model**：换模型不许让缓存失效，那只会逼人把同一句话重烧一遍。
/// 版本号由调用方传入而不是在函数里写死：字幕与改名两套 prompt 各涨各的号，互不作废。
pub fn hash(prompt_version: &str, content: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(prompt_version.as_bytes());
    h.update(content);
    h.finalize().to_hex().to_string()
}

/// 源文件的指纹（mtime + size）。比每次重算内容哈希便宜，够判断「源没动过」。
#[derive(Debug, Clone, Copy)]
pub struct FileStat {
    pub mtime: f64,
    pub size: u64,
}

pub fn stat(path: &str) -> Option<FileStat> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Some(FileStat { mtime, size: md.len() })
}

/// mtime 近似相等（浮点秒走库再回来会有误差）。`subtitle_exempt` 与
/// `ai_translated` 的现状分类共用同一条判定。
pub(crate) fn mtime_close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.001
}

/// 该字幕是否「已翻译可豁免」：命中账（mtime/size 一致）或译文文件比源新，且译文仍在盘上。
/// 既覆盖新库（账里记的），也覆盖老用户（只有 .zh.* 没有账）—— 二者都不该再翻一遍。
pub fn subtitle_exempt(conn: &Connection, source: &str, out_path: &str, st: &FileStat) -> bool {
    // 1) 账里命中且源指纹没变 + 译文还在 → 豁免
    {
        let mut stmt = match conn.prepare(
            "SELECT file_mtime, file_size, out_path FROM ai_translate_cache \
             WHERE source_path = ?1 AND kind = 'subtitle'",
        ) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let rows = stmt
            .query_map([source], |r| {
                Ok((r.get::<_, Option<f64>>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<String>>(2)?))
            })
            .ok();
        if let Some(rows) = rows {
            for r in rows.flatten() {
                let (Some(mtime), Some(size), Some(out)) = r else { continue };
                if mtime_close(mtime, st.mtime) && size == st.size as i64 && Path::new(&out).is_file() {
                    return true;
                }
            }
        }
    }
    // 2) 兜底：译文文件存在且不比源旧（老库没有账也能现场判）。
    Path::new(out_path).is_file()
        && stat(out_path).map(|o| o.mtime >= st.mtime).unwrap_or(false)
}

/// 这条字幕有没有配对的音频/视频文件（记录性字段，不参与豁免判定）。
/// base = 去字幕扩展后的文件名，在**同目录**里找同名的媒体文件。
pub fn audio_of(subtitle: &str) -> Option<String> {
    let dir = subtitle.rsplit_once(['/', '\\']).map(|(d, _)| d).unwrap_or("");
    let name = subtitle.rsplit_once('/').map(|(_, n)| n).unwrap_or(subtitle);
    let name = name.rsplit_once('\\').map(|(_, n)| n).unwrap_or(name);
    let base = name.rsplit_once('.').map(|(b, _)| b.to_string()).unwrap_or_default();
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        let stem = n.rsplit_once('.').map(|(s, _)| s).unwrap_or(&n);
        let ext = n.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
        (stem == base && MEDIA_EXTS.contains(&ext.as_str()))
            .then(|| crate::ai_prompt::join_path(dir, &n))
    })
}

/// 记一条字幕翻译。失败就回退到只缓存内容——不能因为记账失败挡了翻译主流程。
pub fn record_subtitle(
    conn: &Connection,
    source: &str,
    out_path: &str,
    st: &FileStat,
    content_hash: &str,
    audio: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO ai_translate_cache \
         (kind, source_path, content_hash, file_mtime, file_size, out_path, audio_path, translated_at) \
         VALUES ('subtitle', ?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(source_path) DO UPDATE SET \
         content_hash=excluded.content_hash, file_mtime=excluded.file_mtime, \
         file_size=excluded.file_size, out_path=excluded.out_path, \
         audio_path=excluded.audio_path, translated_at=excluded.translated_at",
        rusqlite::params![source, content_hash, st.mtime, st.size as i64, out_path, audio, now_ms()],
    )?;
    Ok(())
}

/// 读复用缓存并顺带刷新 last_used（同一段原文诊断后从缓存取，缓存就该知道它还被用着）。
pub fn get_content(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT content FROM ai_subtitle_content WHERE content_hash = ?1")?;
    let first = stmt.query_map([key], |r| r.get::<_, String>(0))?.next();
    match first {
        Some(Ok(content)) => {
            conn.execute(
                "UPDATE ai_subtitle_content SET last_used = ?1 WHERE content_hash = ?2",
                rusqlite::params![now_ms(), key],
            )?;
            Ok(Some(content))
        }
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// 写入复用缓存，超过上限就删最旧（LRU 按 last_used 淘汰，「压缩」= 相同内容只存一条）。
pub fn put_content(conn: &Connection, key: &str, content: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO ai_subtitle_content (content_hash, content, size, last_used) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(content_hash) DO UPDATE SET content=excluded.content, size=excluded.size, last_used=excluded.last_used",
        rusqlite::params![key, content, content.len() as i64, now_ms()],
    )?;
    let cnt: i64 = conn.query_row("SELECT COUNT(*) FROM ai_subtitle_content", [], |r| r.get(0))?;
    if cnt > CONTENT_CACHE_CAP {
        conn.execute(
            "DELETE FROM ai_subtitle_content WHERE content_hash IN \
             (SELECT content_hash FROM ai_subtitle_content ORDER BY last_used ASC LIMIT ?1)",
            [cnt - CONTENT_CACHE_CAP],
        )?;
    }
    Ok(())
}

/// 该字幕文件是否「已翻译可豁免」。扫描命令对每个扫到的字幕调它回填 `translated`。
pub fn subtitle_cached(conn: &Connection, path: &str) -> bool {
    let Some(st) = stat(path) else { return false };
    let Some((_, ext)) = path.rsplit_once('.') else { return false };
    let out = crate::ai_prompt::translated_path(path, &ext.to_ascii_lowercase());
    subtitle_exempt(conn, path, &out, &st)
}

// ── 明文放行名单（P0-3 的显式例外，007 迁移）────────────────────────────
//
// 默认策略（ai_url）拒绝公网明文与非 443 端口；自建反代（http://公网IP:自定义端口）
// 是正当场景，但放行必须是**用户的显式动作**，且落在 SQLite 而不是可被整份覆盖的
// 设置 JSON —— 同步盘/备份还原不该能悄悄带上一条放行。进程内缓存一份：策略检查
// 在请求热路径上，不能每块请求都开一次库；登记/撤销（`set_plaintext_allow`）负责刷新。

static PLAINTEXT_ALLOW: std::sync::OnceLock<parking_lot::RwLock<Option<std::sync::Arc<Vec<String>>>>> =
    std::sync::OnceLock::new();

/// 当前放行名单（`host:port` 形状，见 `ai_url::endpoint_key`）。首次访问从库加载，
/// 之后走缓存；读库失败按**空名单**处理并记日志 —— 宁可退回默认策略，不能悄悄放行。
pub fn plaintext_allowlist() -> Vec<String> {
    let cache = PLAINTEXT_ALLOW.get_or_init(|| parking_lot::RwLock::new(None));
    if let Some(list) = cache.read().as_ref() {
        return list.as_ref().clone();
    }
    refresh_plaintext_cache()
}

/// 登记 / 撤销一条放行，返回刷新后的完整名单（前端据此重画）。
/// `endpoint` 形状 = `host:port`（IPv6 带方括号）；这里重新解析归一化，不信任
/// 调用方拼好的字符串。本机/局域网地址拒绝登记 —— 它们本来就放行，名单只会误导人。
pub fn set_plaintext_allow(endpoint: &str, allow: bool) -> Result<Vec<String>, String> {
    let (host, port) = parse_endpoint(endpoint)?;
    if crate::ai_url::is_local_host(&host) {
        return Err("本机/局域网地址本来就允许明文，无需放行".into());
    }
    let db = crate::paths::library_db_path();
    crate::db::with_db_text(&db, |c| set_plaintext_allow_conn(c, &host, port, allow).map_err(|e| e.to_string()))
        .map_err(|e| format!("写放行名单失败: {e}"))?;
    Ok(refresh_plaintext_cache())
}

/// 表读写（拆出来给内存库测）：登记走 upsert，撤销是删除。
fn set_plaintext_allow_conn(conn: &Connection, host: &str, port: u16, allow: bool) -> rusqlite::Result<()> {
    if allow {
        conn.execute(
            "INSERT INTO ai_plaintext_allow (host, port, confirmed_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(host, port) DO UPDATE SET confirmed_at=excluded.confirmed_at",
            rusqlite::params![host, port as i64, now_ms()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM ai_plaintext_allow WHERE host = ?1 AND port = ?2",
            rusqlite::params![host, port as i64],
        )?;
    }
    Ok(())
}

/// `host:port` → 归一化的 (host, port)。必须带显式端口（省端口时补出来的默认值
/// 十有八九不是用户真实端点）；IPv6 的 host_str 自带方括号，剥成裸 host ——
/// 与 `ai_url::endpoint_key`（重新按需加括号）共用同一形状。
fn parse_endpoint(endpoint: &str) -> Result<(String, u16), String> {
    let url = reqwest::Url::parse(&format!("http://{}/", endpoint.trim()))
        .map_err(|_| format!("放行目标形状不对：{endpoint}（应为 host:port）"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("放行目标缺少主机：{endpoint}"))?
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    let port = url.port().ok_or_else(|| format!("放行目标必须带端口：{endpoint}"))?;
    Ok((host, port))
}

fn load_plaintext_allowlist(conn: &Connection) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare("SELECT host, port FROM ai_plaintext_allow ORDER BY confirmed_at") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else {
        return Vec::new();
    };
    rows.flatten().map(|(h, p)| crate::ai_url::endpoint_key(&h, p as u16)).collect()
}

fn refresh_plaintext_cache() -> Vec<String> {
    let db = crate::paths::library_db_path();
    let list = crate::db::with_db_text(&db, |c| Ok(load_plaintext_allowlist(c))).unwrap_or_else(|e| {
        crate::paths::append_log(&format!("[ai] 读放行名单失败，按空名单处理: {e}"));
        Vec::new()
    });
    let cache = PLAINTEXT_ALLOW.get_or_init(|| parking_lot::RwLock::new(None));
    *cache.write() = Some(std::sync::Arc::new(list.clone()));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::initialize_schema;
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        initialize_schema(&conn).expect("建表");
        conn
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("kx-ai-cache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("临时目录");
        p
    }

    #[test]
    fn hash_is_stable_and_carries_the_prompt_version() {
        let v = crate::ai_prompt::SUBTITLE_PROMPT_VERSION;
        assert_eq!(hash(v, b""), hash(v, b""));
        assert_eq!(
            hash(v, "[00:01.00]abc".as_bytes()),
            hash(v, "[00:01.00]abc".as_bytes())
        );
        assert_eq!(hash(v, b"abc").len(), 64, "blake3 的十六进制是 64 位");
        assert_ne!(hash(v, b"abc"), hash(v, b"abd"));
        // 一致性账的全部意义：措辞一变（版本号一涨），旧键整体不再命中。
        assert_ne!(hash(v, b"abc"), hash("subtitle-2", b"abc"), "提示词版本进键");
    }

    #[test]
    fn recorded_subtitle_is_exempt_when_fingerprint_matches_and_out_exists() {
        let dir = tmpdir("exempt");
        let src = dir.join("a.lrc");
        let out = crate::ai_prompt::translated_path(src.to_str().unwrap(), "lrc");
        // 译文先写、源后写：源严格比译文新，兜底规则不豁免，只能靠账。
        std::fs::write(&out, "[00:01.00]\u{7ffb}\u{8bd1}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(25));
        std::fs::write(&src, "[00:01.00]x").unwrap();
        let st = stat(src.to_str().unwrap()).unwrap();

        let conn = db();
        assert!(!subtitle_exempt(&conn, src.to_str().unwrap(), &out, &st), "源比译文新又没记账 → 不豁免");

        record_subtitle(
            &conn,
            src.to_str().unwrap(),
            &out,
            &st,
            &hash(crate::ai_prompt::SUBTITLE_PROMPT_VERSION, b"x"),
            None,
        )
        .unwrap();
        assert!(subtitle_exempt(&conn, src.to_str().unwrap(), &out, &st), "账对 + 译文在 → 豁免");

        // 源文件一动（mtime/size），不再豁免
        std::fs::write(&src, "[00:01.00]x2").unwrap();
        let st2 = stat(src.to_str().unwrap()).unwrap();
        assert!(!subtitle_exempt(&conn, src.to_str().unwrap(), &out, &st2), "源变了就要重翻");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fallback_exempts_when_out_is_newer_even_without_a_ledger_row() {
        let dir = tmpdir("legacy");
        let src = dir.join("a.lrc");
        std::fs::write(&src, "[00:01.00]x").unwrap();
        let out = crate::ai_prompt::translated_path(src.to_str().unwrap(), "lrc");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&out, "done").unwrap();
        let st = stat(src.to_str().unwrap()).unwrap();
        let conn = db();
        assert!(subtitle_exempt(&conn, src.to_str().unwrap(), &out, &st), "老用户现成的 .zh 也豁免");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_cache_reuses_and_evicts_oldest() {
        let conn = db();
        put_content(&conn, "k1", "one").unwrap();
        put_content(&conn, "k2", "two").unwrap();
        assert_eq!(get_content(&conn, "k1").unwrap().as_deref(), Some("one"));
        assert_eq!(get_content(&conn, "missing").unwrap(), None);
        put_content(&conn, "k1", "one-v2").unwrap();
        assert_eq!(get_content(&conn, "k1").unwrap().as_deref(), Some("one-v2"), "同键覆盖");

        // 超出上限：最旧的被淘汰。先塞满到上限再插一个新键，最旧的 k1 应被挤掉。
        for i in 0..CONTENT_CACHE_CAP {
            put_content(&conn, &format!("f{i:03}"), "x").unwrap();
        }
        put_content(&conn, "new", "y").unwrap();
        assert_eq!(get_content(&conn, "new").unwrap().as_deref(), Some("y"));
        assert_eq!(get_content(&conn, "f000").unwrap(), None, "最旧应被淘汰");
    }

    #[test]
    fn audio_of_pairs_subtitle_with_same_stem_media() {
        let dir = tmpdir("audio");
        std::fs::write(dir.join("track.mp3"), b"x").unwrap();
        std::fs::write(dir.join("track.lrc"), b"x").unwrap();
        let a = audio_of(&format!("{}/track.lrc", dir.to_str().unwrap()));
        assert_eq!(a.as_deref(), Some(format!("{}/track.mp3", dir.to_str().unwrap())).as_deref());
        assert!(audio_of(&format!("{}/nope.lrc", dir.to_str().unwrap())).is_none(), "无配对媒体");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 放行名单的表读写：登记 upsert、撤销删除、读出按 endpoint_key 归一化。
    /// 与 `ai_url::check_with` 对一遍：名单里的条目真能解锁那个端点。
    #[test]
    fn plaintext_allow_rows_roundtrip_and_unlock_the_endpoint() {
        let conn = db();
        set_plaintext_allow_conn(&conn, "125.216.220.3", 8788, true).unwrap();
        set_plaintext_allow_conn(&conn, "125.216.220.3", 8788, true).unwrap(); // 重复登记 = upsert，不炸
        let list = load_plaintext_allowlist(&conn);
        assert_eq!(list, vec!["125.216.220.3:8788".to_string()]);
        assert!(crate::ai_url::check_with("http://125.216.220.3:8788/v1", &list).is_ok(), "名单要能解锁端点");

        set_plaintext_allow_conn(&conn, "125.216.220.3", 8788, false).unwrap();
        assert!(load_plaintext_allowlist(&conn).is_empty(), "撤销即删除");
        assert!(crate::ai_url::check_with("http://125.216.220.3:8788/v1", &load_plaintext_allowlist(&conn)).is_err());
    }

    /// endpoint 归一化：必须带显式端口；大小写与尾随斜杠都收干净；垃圾形状如实拒。
    #[test]
    fn endpoint_parsing_is_strict() {
        assert_eq!(parse_endpoint("EXAMPLE.com:8788/").unwrap(), ("example.com".into(), 8788));
        assert_eq!(parse_endpoint("[::1]:8788").unwrap(), ("::1".into(), 8788));
        assert!(parse_endpoint("example.com").is_err(), "缺端口的默认值不是用户的真实端点");
        assert!(parse_endpoint("不是地址").is_err());
    }
}