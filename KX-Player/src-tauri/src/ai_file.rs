//! 单个字幕文件的翻译闭环：豁免判断 → 缓存复用 → AI 翻译 → 写盘 → 记账。
//!
//! 从 `ai_chunk.rs` 拆出来同样是为了守 Rust 单文件 ≤400 行的红线（第 12 批）：
//! 「一批文本怎么翻」留在 ai_chunk，「一个文件怎么落盘记账」在这里。
//! 口径三条：
//! - 「已翻译豁免」与「内容复用缓存」都是省钱闸门，`force` 重译必须同时绕开两道 ——
//!   否则点「重译」只是把旧的（可能混着原文的）译文再抄一遍，重译就名存实亡；
//! - 内容缓存只收**零回退**的干净结果：混着原文的译文一旦进缓存，同内容的文件永远
//!   拿到那份坏译文，坏结果就洗不掉了；
//! - 回退行数原样上报：file-done 的 `fallbackLines` 是「翻完了」与「翻好了」的差值。

use crate::ai_chunk::{translate_chunks, Run};
use crate::ai_event::Loc;
use crate::ai_prompt as prompt;
use crate::subtitles as subs;

/// 一个文件翻译完的产物：`.zh` 路径 + 本次回退原文的行数 + 总段数。
/// 豁免/缓存命中的路径 `fallback_lines` 为 0（没有新翻的行）。
pub struct TranslatedFile {
    pub out_path: String,
    pub fallback_lines: usize,
    pub segment_count: usize,
}

/// 翻译单个字幕文件。Ok(None) = 被取消。带「已翻译豁免 + 复用缓存」：
/// - 源文件没变且 .zh 译文还在 → 直接复用（不读内容、不烧 token）；
/// - 原文内容命中复用缓存 → 把缓存里那份译文写出来（也不用烧 token）；
/// - 否则走 AI，翻完把「源指纹账」落库，干净结果再进「内容缓存」。
///
/// `force`（行内「重译」）跳过前两道省钱闸门，逼一次真翻译。
pub async fn translate_one(
    run: &Run,
    loc: Loc<'_>,
    path: &str,
    force: bool,
) -> Result<Option<TranslatedFile>, String> {
    // 译文文件不进翻译管线：扫描已在源头排除 .zh.*（ai_scan），但旧扫描结果还挂在
    // 界面上、用户不重扫就点「开始翻译」时，路径照样会递进来 —— 这里兜住最后一道，
    // 报错让人看见，而不是悄悄翻出 1.zh.zh.lrc。
    if prompt::is_translated_output(path) {
        return Err("这是译文文件（*.zh.*），不该再翻译；重新扫描后队列里不会再有它".into());
    }
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let out_path = prompt::translated_path(path, &ext);
    let db = crate::paths::library_db_path();

    // 0) 已翻译可豁免：源没变 + .zh 译文还在 → 直接复用现成的 .zh.*，不重翻。
    if !force
        && crate::ai_cache::stat(path)
            .map(|st| crate::db::with_db_text(&db, |c| Ok(crate::ai_cache::subtitle_exempt(c, path, &out_path, &st))).unwrap_or(false))
            .unwrap_or(false)
    {
        return Ok(Some(TranslatedFile { out_path, fallback_lines: 0, segment_count: 0 }));
    }

    let content = crate::limits::read_text_capped(std::path::Path::new(path), crate::limits::MAX_TEXT_BYTES)
        .map_err(|e| format!("读取失败: {e}"))?;
    let content_hash = crate::ai_cache::hash(prompt::SUBTITLE_PROMPT_VERSION, content.as_bytes());

    // 1) 内容命中复用缓存：译文直接来自缓存，跳过 AI。
    if !force {
        let cached: Option<String> =
            crate::db::with_db_text(&db, |c| crate::ai_cache::get_content(c, &content_hash).map_err(|e| e.to_string()))
                .map_err(|e| format!("读复用缓存失败: {e}"))?;
        if let Some(rendered) = cached {
            crate::atomic::write_str(std::path::Path::new(&out_path), &rendered).map_err(|e| format!("写入失败: {e}"))?;
            let _ = record_lookup(path, &out_path, &content_hash);
            return Ok(Some(TranslatedFile { out_path, fallback_lines: 0, segment_count: 0 }));
        }
    }

    let doc = subs::parse(&content, &ext).ok_or_else(|| format!("不支持的字幕格式: .{ext}"))?;
    if doc.segment_count() == 0 {
        return Err("没有可翻译的文本".into());
    }
    let (translated, fallback) = match translate_chunks(run, loc, doc.segments()).await {
        Some(t) => t,
        None => return Ok(None),
    };
    let rendered = doc.render(&translated);
    crate::atomic::write_str(std::path::Path::new(&out_path), &rendered).map_err(|e| format!("写入失败: {e}"))?;
    // 记账照旧（译文确实写出来了）；内容缓存只收零回退的干净结果。
    // 两步失败都只回退到「少了一级缓存」，不把翻译主流程挡住。
    let _ = record_lookup(path, &out_path, &content_hash);
    if fallback == 0 {
        let _ = crate::db::with_db_text(&db, |c| {
            crate::ai_cache::put_content(c, &content_hash, &rendered).map_err(|e| format!("写内容缓存失败: {e}"))
        });
    }
    Ok(Some(TranslatedFile { out_path, fallback_lines: fallback, segment_count: doc.segment_count() }))
}

/// 把「源文件 → 这次翻出来的 .zh」记进豁免账，附上配对的音频路径。
fn record_lookup(path: &str, out_path: &str, content_hash: &str) -> Result<(), String> {
    let db = crate::paths::library_db_path();
    let audio = crate::ai_cache::audio_of(path);
    let Some(st) = crate::ai_cache::stat(path) else {
        return Ok(()); // 源已被删，账记不记都无所谓
    };
    crate::db::with_db_text(&db, |c| {
        crate::ai_cache::record_subtitle(c, path, out_path, &st, content_hash, audio.as_deref())
            .map_err(|e| format!("记录翻译账失败: {e}"))
    })
}
