//! AI 翻译的「已翻译账」与「内容复用缓存」（AI-15「已处理项默认跳过」）。
//!
//! 为什么要有这一层：翻译一次要烧 token，重复翻译同一个文件 / 同一段字幕是纯浪费。
//! 这里把「翻译过什么」落库（`ai_translate_cache`），再次扫描时命中且源文件没变、译文
//! 还在 → 默认跳过；再把「原文内容 → 译文」复用缓存（`ai_subtitle_content`）按
//! `blake3(提示词版本 + 原文)` 去重，相同内容的多个文件共一条缓存、且有容量上限（LRU 淘汰）——「合理选择和压缩」。
//!
//! 本文件只管读库/写库的纯函数，连接由调用方（命令 / 翻译任务）经 `db::with_db_text`
//! 注入；缓存键归一化用 crate::model::normalize_path，避免正反斜杠一双键。

use rusqlite::Connection;
use std::path::Path;

/// 字幕在扫描里也会被当译文输出再次扫到，`is_translated_subtitle` 只对源文件判；
/// 这里不碰撞它。缓存守卫规则见 `subtitle_exempt`。
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

fn mo_close(a: f64, b: f64) -> bool {
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
                if mo_close(mtime, st.mtime) && size == st.size as i64 && Path::new(&out).is_file() {
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

/// 已生成的文件夹名译名（按目录路径）。给「生成译名」点第二次时复用，不烧 token。
pub fn load_folder_suggestions(conn: &Connection) -> rusqlite::Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT source_path, result_text FROM ai_translate_cache WHERE kind = 'folder' AND result_text IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for r in rows {
        let (path, text) = r?;
        out.insert(path, text);
    }
    Ok(out)
}

/// 记一条文件夹名译名建议。
pub fn record_folder(conn: &Connection, path: &str, suggestion: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO ai_translate_cache (kind, source_path, result_text, translated_at) \
         VALUES ('folder', ?1, ?2, ?3) \
         ON CONFLICT(source_path) DO UPDATE SET result_text=excluded.result_text, translated_at=excluded.translated_at",
        rusqlite::params![path, suggestion, now_ms()],
    )?;
    Ok(())
}

/// 该字幕文件是否「已翻译可豁免」。扫描命令对每个扫到的字幕调它回填 `translated`。
pub fn subtitle_cached(conn: &Connection, path: &str) -> bool {
    let Some(st) = stat(path) else { return false };
    let Some((_, ext)) = path.rsplit_once('.') else { return false };
    let out = crate::ai_prompt::translated_path(path, &ext.to_ascii_lowercase());
    subtitle_exempt(conn, path, &out, &st)
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
    fn folder_suggestion_is_recorded_and_loaded() {
        let conn = db();
        record_folder(&conn, "C:/主目录", "Main").unwrap();
        record_folder(&conn, "C:/子", "Sub").unwrap();
        let map = load_folder_suggestions(&conn).unwrap();
        assert_eq!(map.get("C:/主目录").map(String::as_str), Some("Main"));
        assert!(map.contains_key("C:/主目录"));
        assert_eq!(map.len(), 2);
        record_folder(&conn, "C:/主目录", "MainV2").unwrap();
        assert_eq!(
            load_folder_suggestions(&conn).unwrap().get("C:/主目录").map(String::as_str),
            Some("MainV2"),
            "覆盖更新"
        );
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
}