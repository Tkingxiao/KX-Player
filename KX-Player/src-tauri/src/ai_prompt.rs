//! AI 翻译的「说话方式」：提示词、切块、回文解析、输出路径。
//!
//! 这些函数原来散在前端 src/services/aiTranslate.ts 里，B3 把整条字幕翻译链路搬到 Rust 后
//! 跟着过来。**逐字保留**原来的提示词与「80% 命中才算成功」的判据 —— 换个说法译文风格就变，
//! 那是产品决定，不是重构顺手能改的。

/// 送模型的系统提示词（与前端旧实现逐字一致）
pub const SYSTEM_PROMPT: &str = "你是专业的音声字幕/歌词翻译器。用户会给出若干「编号. 文本」行。把每一行翻译成自然、口语化的简体中文：保持编号不变、逐行输出；保留原文中的★♪【】※等装饰符号与表情；拟声词可音译或保留；只输出译文，不要解释、不要合并行。";

/// 拼接目录与文件名（统一用正斜杠：前端与 mpv 都按这个约定消费路径）
pub(crate) fn join_path(dir: &str, name: &str) -> String {
    let mut base = dir;
    while base.ends_with('/') || base.ends_with('\\') {
        base = &base[..base.len() - 1];
    }
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// 译文输出路径：同目录、名字后加 .zh（x.srt → x.zh.srt）
pub fn translated_path(src: &str, ext: &str) -> String {
    let (dir, name) = match src.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => src.rsplit_once('\\').unwrap_or(("", src)),
    };
    let base = name.rsplit_once('.').map(|(b, _)| b).unwrap_or(name);
    join_path(dir, &format!("{base}.zh.{ext}"))
}

/// 按块大小切段，返回每块的段下标区间 [start, end)
pub fn chunk_ranges(total: usize, chunk_size: usize) -> Vec<(usize, usize)> {
    let size = chunk_size.max(1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < total {
        let end = (i + size).min(total);
        out.push((i, end));
        i = end;
    }
    out
}

/// 「1. 文本」编号提示词（与前端逐字一致，模型按这个格式回）
pub fn numbered_prompt(texts: &[String]) -> String {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {t}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 解析模型回文里的「编号. 译文」。命中不足 80% 视为整块失败（None → 保留原文重试/回退）。
/// 同一编号出现多次时**取第一次**，编号越界忽略。
pub fn parse_numbered_reply(reply: &str, expected: usize) -> Option<Vec<String>> {
    let mut got: Vec<Option<String>> = vec![None; expected];
    for line in crate::subtitles::split_lines(reply) {
        let l = line.trim_start();
        let digits: String = l.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        let rest = l[digits.len()..].trim_start();
        let mut it = rest.char_indices();
        let Some((_, sep)) = it.next() else { continue };
        if !matches!(sep, '.' | '、' | ')' | '）' | '：' | ':') {
            continue;
        }
        let text = rest[sep.len_utf8()..].trim();
        if text.is_empty() {
            continue;
        }
        if let Ok(n) = digits.parse::<usize>() {
            if n >= 1 && n <= expected && got[n - 1].is_none() {
                got[n - 1] = Some(text.to_string());
            }
        }
    }
    let hit = got.iter().filter(|v| v.is_some()).count();
    if hit < (expected * 8).div_ceil(10) {
        return None;
    }
    Some(got.into_iter().map(|v| v.unwrap_or_default()).collect())
}

/// 空译文回退原文（与前端一致：宁可留原文，也不要空行）
pub fn fill_missing(translated: Vec<String>, origin: &[String]) -> Vec<String> {
    translated
        .into_iter()
        .enumerate()
        .map(|(i, t)| if t.is_empty() { origin.get(i).cloned().unwrap_or_default() } else { t })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_ranges_covers_everything_once() {
        assert_eq!(chunk_ranges(0, 15), Vec::<(usize, usize)>::new());
        assert_eq!(chunk_ranges(3, 15), vec![(0, 3)]);
        assert_eq!(chunk_ranges(30, 15), vec![(0, 15), (15, 30)]);
        assert_eq!(chunk_ranges(31, 15), vec![(0, 15), (15, 30), (30, 31)]);
        assert_eq!(chunk_ranges(2, 0), vec![(0, 1), (1, 2)], "chunk_size 0 不能死循环");
    }

    #[test]
    fn prompt_is_numbered_from_one() {
        let p = numbered_prompt(&["甲".into(), "乙".into()]);
        assert_eq!(p, "1. 甲\n2. 乙");
    }

    #[test]
    fn reply_needs_eighty_percent_of_the_lines() {
        // 阈值与前端 Math.ceil(expected * 0.8) 等价：5 → 4、10 → 8
        assert!(parse_numbered_reply("1. a\n2. b\n3. c\n4. d\n5. e", 5).is_some());
        assert!(parse_numbered_reply("1. a\n2. b\n3. c\n4. d", 5).is_some(), "4/5 恰好在阈值上");
        assert!(parse_numbered_reply("1. a\n2. b\n3. c", 5).is_none(), "3/5 低于阈值");
        assert!(
            parse_numbered_reply("1. a\n2. b\n3. c\n4. d\n5. e\n6. f\n7. g\n8. h", 10).is_some(),
            "8/10 恰好在阈值上（ceil(10*0.8)=8）"
        );
        assert!(
            parse_numbered_reply("1. a\n2. b\n3. c\n4. d\n5. e\n6. f\n7. g", 10).is_none(),
            "7/10 低于阈值"
        );
        assert!(parse_numbered_reply("1. a\n2. b\n3. c\n4. d\n5. e\n6. f\n7. g\n8. h\n9. i\n10. j", 10).is_some());
    }

    #[test]
    fn reply_keeps_first_hit_and_ignores_out_of_range() {
        let reply = "1. 第一次\n1. 第二次\n9. 越界\n2. 乙";
        let out = parse_numbered_reply(reply, 2).unwrap();
        assert_eq!(out, vec!["第一次".to_string(), "乙".to_string()]);
    }

    #[test]
    fn translated_path_adds_zh_before_extension() {
        assert_eq!(translated_path("/a/b/x.srt", "srt"), "/a/b/x.zh.srt");
        assert_eq!(translated_path("/a/b/y", "lrc"), "/a/b/y.zh.lrc");
        assert_eq!(translated_path("z.lrc", "lrc"), "z.zh.lrc");
        assert_eq!(join_path("/a/b/", "c"), "/a/b/c");
    }
}
