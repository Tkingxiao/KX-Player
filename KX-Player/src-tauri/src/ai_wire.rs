//! AI-16 半自动模式的**文本格式**：`prompts.txt` 怎么写、`translations.txt` 怎么读回来。
//!
//! 分隔符只有 `### <数字> ###` 一种（`04 §7.13`）：`=` `-` `#` 都可能出现在字幕原文里，只有
//! 「独占一行 + 中间是纯数字」切得开、也不会跟正文撞。批次从哪来、要不要写盘在 `ai_export.rs`。
//!
//! 序号是这份格式**唯一的对齐依据**：导入不看顺序，所以用户可以重排、可以几台机器各跑一段
//! 再拼起来（`04 §7.13` 的「分机器并行跑同一批」）。

use crate::subtitles as subs;

/// 一行「### 7 ###」里的序号；不是这个形状就返回 None
pub fn header_seq(line: &str) -> Option<usize> {
    let l = line.trim();
    let digits = l.strip_prefix("### ")?.strip_suffix(" ###")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// 原文自带分隔线形状的行 → 这一条不能进导出批次：它会把用户那侧的切块打成错位的两块。
pub fn safe_to_export(text: &str) -> bool {
    !subs::split_lines(text).iter().any(|l| header_seq(l).is_some())
}

/// 导出文本：每段一块 `### <序号> ###` + 原文，块间空一行。
/// SYSTEM 不在里面 —— 用户自己把提示词配给手边的工具（`04 §7.13`），软件只负责切得开、对得回。
pub fn build_prompt<'a>(items: impl Iterator<Item = (usize, &'a str)>) -> String {
    let mut s = String::new();
    for (seq, text) in items {
        s.push_str(&format!("### {seq} ###\n{text}\n\n"));
    }
    s
}

/// 导入文本切出来的块：序号 → 译文，外加三个计数
#[derive(Default, Debug)]
pub struct Blocks {
    /// 同一序号**取第一次**（后面的算重复，不覆盖）
    pub map: std::collections::HashMap<usize, String>,
    /// 块头总数（含空的、越界的）
    pub total: usize,
    pub duplicates: usize,
    pub empty: usize,
}

/// 按 `### N ###` 分块，块内全部内容就是译文（内部换行留给 `fit` 按字幕格式压平）。
/// 第一个块头之前的内容（工具顺手写的说明）整段忽略。
pub fn parse_blocks(text: &str) -> Blocks {
    let mut out = Blocks::default();
    let mut seq: Option<usize> = None;
    let mut body: Vec<&str> = Vec::new();
    for line in subs::split_lines(text) {
        match header_seq(line) {
            Some(n) => {
                close(&mut out, seq, &body);
                seq = Some(n);
                body.clear();
            }
            None => {
                if seq.is_some() {
                    body.push(line);
                }
            }
        }
    }
    close(&mut out, seq, &body);
    out
}

fn close(out: &mut Blocks, seq: Option<usize>, body: &[&str]) {
    let Some(n) = seq else { return };
    out.total += 1;
    let text = body.join("\n").trim().to_string();
    if text.is_empty() {
        out.empty += 1;
        return;
    }
    if out.map.contains_key(&n) {
        out.duplicates += 1;
        return;
    }
    out.map.insert(n, text);
}

/// LRC 的译文必须压回一行（它要拼进时间戳后面，换行会把文件结构打断）；SRT/VTT 保留多行。
pub fn fit(t: &str, kind: subs::Kind) -> String {
    if kind == subs::Kind::Lrc {
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_needs_the_exact_shape() {
        assert_eq!(header_seq("### 12 ###"), Some(12));
        assert_eq!(header_seq("  ### 7 ###  "), Some(7));
        assert_eq!(header_seq("### 12"), None);
        assert_eq!(header_seq("###12 ###"), None);
        assert_eq!(header_seq("### 十二 ###"), None);
        assert_eq!(header_seq("# 1 #"), None);
        assert_eq!(header_seq("--- 1 ---"), None);
        // 0 不是合法批次号，但它该被「序号越界」抓住而不是被当成正文吞掉
        assert_eq!(header_seq("### 0 ###"), Some(0));
    }

    #[test]
    fn prompt_blocks_are_blank_line_separated() {
        let p = build_prompt(vec![(1, "你好"), (2, "世界")].into_iter());
        assert_eq!(p, "### 1 ###\n你好\n\n### 2 ###\n世界\n\n");
        assert!(!p.contains(crate::ai_prompt::SYSTEM_PROMPT), "SYSTEM 不进导出文件");
    }

    #[test]
    fn source_lines_that_look_like_the_separator_are_flagged() {
        assert!(safe_to_export("普通一行"));
        assert!(!safe_to_export("### 3 ###"), "整行就是分隔符");
        assert!(!safe_to_export("上面一行\n### 9 ###\n下面一行"), "多行 cue 里藏了一行分隔符");
        assert!(safe_to_export("# 不是 ###\n### 也不是 ###?"), "形状不完全对上就不算");
    }

    #[test]
    fn blocks_align_by_index_and_survive_shuffling() {
        let b = parse_blocks("### 3 ###\n丙\n\n### 1 ###\n甲\n");
        assert_eq!((b.total, b.map.len()), (2, 2));
        assert_eq!(b.map[&1], "甲");
        assert_eq!(b.map[&3], "丙");
    }

    #[test]
    fn first_wins_on_duplicates_and_empty_blocks_are_dropped() {
        let b = parse_blocks("说明文字\n### 1 ###\n第一个\n### 1 ###\n第二个\n### 2 ###\n  \n");
        assert_eq!((b.total, b.map.len(), b.duplicates, b.empty), (3, 1, 1, 1));
        assert_eq!(b.map[&1], "第一个");
        assert!(!b.map.contains_key(&2), "空块不算译文");
    }

    #[test]
    fn multi_line_blocks_keep_their_shape_per_format() {
        assert_eq!(fit("上\n阙", subs::Kind::Lrc), "上 阙");
        assert_eq!(fit("上\n阙", subs::Kind::Srt), "上\n阙");
    }
}
