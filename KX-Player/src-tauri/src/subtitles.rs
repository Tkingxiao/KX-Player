//! 字幕文本层：LRC / SRT / VTT 的解析与回写。
//!
//! 为什么字幕文本解析要在 Rust 里：B3 要求「字幕翻译的逐块 await 与前端 api.listDir 递归扫
//! 目录改为 Rust 侧任务」。逐块翻译一旦在 Rust 里跑，就得在 Rust 里读文件、切段、把译文拼回
//! 原结构 —— 否则每个块还要过一次 IPC，等于没搬。
//!
//! 与前端旧实现（src/services/aiTranslate.ts）的**有意不同**：
//! 1. 旧实现先 parseVttOrSrt 取 cue（**按时间排序 + 按 start 时间去重**），再另起一遍正则按**文件
//!    顺序**收时间段，然后用下标把两者配对 —— 乱序 cue 配上的就是别人的结束时间，而两行同
//!    start 时间戳时其中一行已被去重吞掉，`times[]` 与 cue 数从那里开始整体错位。
//!    这里一次遍历同时拿到「时间段 + 文本」，从根上消掉这类错配。
//! 2. 段落顺序按**文件原序**保留（不排序、不去重），译文与原文 cue 一一对应 —— 翻译不该改写
//!    时间轴顺序，重排是渲染层的事。
//! 3. 这里**不做**字幕渲染（渲染归 mpv/libass，见 B4）：本模块只服务「离线翻译」这条链路。

/// 可翻译的字幕扩展名（与前端 SUBTITLE_EXTS 一致）
pub const SUBTITLE_EXTS: [&str; 3] = ["lrc", "srt", "vtt"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lrc,
    Srt,
    Vtt,
}

impl Kind {
    pub fn from_ext(ext: &str) -> Option<Kind> {
        let e = ext.trim_start_matches('.').to_ascii_lowercase();
        match e.as_str() {
            "lrc" => Some(Kind::Lrc),
            "srt" => Some(Kind::Srt),
            "vtt" => Some(Kind::Vtt),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Lrc => "lrc",
            Kind::Srt => "srt",
            Kind::Vtt => "vtt",
        }
    }
}

/// 解析后的文档：段文本 + 把译文装回原文结构所需的一切
#[derive(Debug, Clone)]
pub struct Doc {
    kind: Kind,
    segments: Vec<String>,
    /// LRC：原文按行留档，line_seg[i] 指向该行对应的段（None = 不翻译的行）
    lines: Vec<String>,
    line_seg: Vec<Option<usize>>,
    /// LRC：行内最后一个时间戳之后、待替换文本的起始字节位置
    line_tail: Vec<usize>,
    /// SRT/VTT：cue 的起止秒与原文
    cues: Vec<Cue>,
}

#[derive(Debug, Clone)]
struct Cue {
    start: f64,
    end: f64,
    text: String,
}

impl Doc {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// 用译文重建整篇文本。译文缺失或为空串时**保留原文**（与前端一致）
    pub fn render(&self, translated: &[String]) -> String {
        match self.kind {
            Kind::Lrc => self.render_lrc(translated),
            k => self.render_cues(translated, k),
        }
    }

    fn render_lrc(&self, translated: &[String]) -> String {
        let mut out: Vec<String> = Vec::with_capacity(self.lines.len());
        for (i, line) in self.lines.iter().enumerate() {
            let Some(seg) = self.line_seg[i] else {
                out.push(line.clone());
                continue;
            };
            let t = translated.get(seg).map(String::as_str).unwrap_or("");
            if t.is_empty() {
                out.push(line.clone());
                continue;
            }
            let cut = self.line_tail[i];
            out.push(format!("{}{t}", &line[..cut]));
        }
        out.join("\n")
    }

    fn render_cues(&self, translated: &[String], kind: Kind) -> String {
        let mut parts: Vec<String> = Vec::new();
        if kind == Kind::Vtt {
            parts.push("WEBVTT".to_string());
        }
        for (i, cue) in self.cues.iter().enumerate() {
            let t = translated.get(i).map(String::as_str).filter(|s| !s.is_empty()).unwrap_or(&cue.text);
            parts.push(format!(
                "{}\n{} --> {}\n{t}",
                i + 1,
                fmt_timestamp(cue.start, kind),
                fmt_timestamp(cue.end, kind)
            ));
        }
        format!("{}\n", parts.join("\n\n"))
    }
}

pub(crate) fn split_lines(s: &str) -> Vec<&str> {
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

fn fmt_timestamp(sec: f64, kind: Kind) -> String {
    let total = if sec.is_finite() && sec > 0.0 { sec } else { 0.0 };
    let h = (total / 3600.0).floor() as i64;
    let m = ((total % 3600.0) / 60.0).floor() as i64;
    let s = (total % 60.0).floor() as i64;
    let ms = ((total - total.floor()) * 1000.0).round() as i64;
    let sep = if kind == Kind::Srt { ',' } else { '.' };
    format!("{h:02}:{m:02}:{s:02}{sep}{ms:03}")
}

/// 解析入口：按扩展名选解析器；扩展名不认识则 None
pub fn parse(content: &str, ext: &str) -> Option<Doc> {
    let kind = Kind::from_ext(ext)?;
    Some(match kind {
        Kind::Lrc => parse_lrc(content),
        Kind::Srt | Kind::Vtt => parse_cues(content, kind),
    })
}

/// 行内的 [MM:SS] / [MM:SS.mmm] 时间戳位置（字节区间）。本 crate 没有 regex 依赖，手写扫描。
fn lrc_timestamp(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        let digits_from = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if !(1..=2).contains(&(j - digits_from)) || j >= b.len() || b[j] != b':' {
            i += 1;
            continue;
        }
        j += 1;
        let sec_from = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j - sec_from != 2 {
            i += 1;
            continue;
        }
        if j < b.len() && b[j] == b'.' {
            j += 1;
            let ms_from = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if !(1..=3).contains(&(j - ms_from)) {
                i += 1;
                continue;
            }
        }
        if j < b.len() && b[j] == b']' {
            out.push((start, j + 1));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn parse_lrc(content: &str) -> Doc {
    let lines: Vec<String> = split_lines(content).into_iter().map(str::to_string).collect();
    let mut segments: Vec<String> = Vec::new();
    let mut line_seg: Vec<Option<usize>> = vec![None; lines.len()];
    let mut line_tail: Vec<usize> = vec![0; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        let stamps = lrc_timestamp(line);
        let Some(&(_, last_end)) = stamps.last() else { continue };
        // 与旧前端一致：去掉全部时间戳后的正文就是待翻译段；空则跳过
        let mut text = String::new();
        let mut cursor = 0;
        for (s, e) in &stamps {
            text.push_str(&line[cursor..*s]);
            cursor = *e;
        }
        text.push_str(&line[cursor..]);
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        line_seg[i] = Some(segments.len());
        // 旧前端的 render 用正则 (\][ ]*)([^\[]+)$ —— 只替换「最后一个 ] 之后且其中没有 [」
        // 的那一段。这里保持同一条件：尾段里还有 [ 就整行不动，避免重复时间戳行被改坏。
        let tail = &line[last_end..];
        line_tail[i] = if tail.contains('[') { line.len() } else { last_end };
        segments.push(text);
    }
    Doc { kind: Kind::Lrc, segments, lines, line_seg, line_tail, cues: Vec::new() }
}

/// 去掉 VTT 内联标签（<c.xxx> / <00:00:00.000> / <b>）
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// 小数部分切分：逗号（SRT）与句点（VTT）都接受
fn split_frac(s: &str) -> (&str, Option<&str>) {
    match (s.find('.'), s.find(',')) {
        (Some(i), Some(j)) => {
            let k = i.min(j);
            (&s[..k], Some(&s[k + 1..]))
        }
        (Some(k), None) | (None, Some(k)) => (&s[..k], Some(&s[k + 1..])),
        (None, None) => (s, None),
    }
}

/// HH:MM:SS,mmm / MM:SS.mmm / HH:MM:SS.mmm → 秒
fn parse_time_token(tok: &str) -> Option<f64> {
    let tok = tok.trim();
    let parts: Vec<&str> = tok.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let (sec_str, frac) = split_frac(parts[parts.len() - 1]);
    let sec = sec_str.trim().parse::<f64>().ok()?;
    let ms = match frac {
        Some(m) => {
            let m = m.trim();
            if m.is_empty() || !m.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            m.chars().chain(std::iter::repeat('0')).take(3).collect::<String>().parse::<f64>().ok()? / 1000.0
        }
        None => 0.0,
    };
    let total = match parts.len() {
        1 => sec + ms,
        2 => parts[0].trim().parse::<f64>().ok()? * 60.0 + sec + ms,
        _ => parts[0].trim().parse::<f64>().ok()? * 3600.0 + parts[1].trim().parse::<f64>().ok()? * 60.0 + sec + ms,
    };
    Some(total)
}

/// 时间轴行：a --> b
fn parse_arrow(line: &str) -> Option<(f64, f64)> {
    let (left, right) = line.split_once("-->")?;
    let start = parse_time_token(left)?;
    let end = parse_time_token(right.split_whitespace().next().unwrap_or(""))?;
    Some((start, end))
}

fn parse_cues(content: &str, kind: Kind) -> Doc {
    let lines = split_lines(content);
    let mut cues: Vec<Cue> = Vec::new();
    let mut i = 0;
    if kind == Kind::Vtt && lines.first().is_some_and(|l| l.trim().to_ascii_uppercase().starts_with("WEBVTT")) {
        i = 1;
    }
    while i < lines.len() {
        let line = lines[i];
        let upper = line.trim_start().to_ascii_uppercase();
        if upper.starts_with("NOTE") || upper.starts_with("STYLE") || upper.starts_with("REGION") {
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() {
                i += 1;
            }
            continue;
        }
        if let Some((start, end)) = parse_arrow(line) {
            let mut text = String::new();
            for next in lines.iter().skip(i + 1) {
                let t = next.trim();
                if t.is_empty() {
                    break;
                }
                if t.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                text = strip_tags(t);
                break;
            }
            if !text.is_empty() {
                cues.push(Cue { start, end, text });
            }
        }
        i += 1;
    }
    let segments = cues.iter().map(|c| c.text.clone()).collect();
    Doc { kind, segments, lines: Vec::new(), line_seg: Vec::new(), line_tail: Vec::new(), cues }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_keeps_metadata_and_replaces_only_timestamped_lines() {
        let src = "[ar:某人]\n[00:01.50]第一句\n[00:05.00]第二句\n";
        let doc = parse(src, "lrc").expect("lrc 可解析");
        assert_eq!(doc.segment_count(), 2, "元数据行不进段落");
        assert_eq!(doc.segments()[0], "第一句");
        let out = doc.render(&["译文一".into(), "译文二".into()]);
        assert!(out.starts_with("[ar:某人]\n"), "非时间戳行原样保留：{out}");
        assert!(out.contains("[00:01.50]译文一"), "时间戳保留、正文替换：{out}");
        assert!(out.contains("[00:05.00]译文二"), "{out}");
    }

    #[test]
    fn lrc_multi_timestamp_line_is_one_segment() {
        let doc = parse("[00:01.00][00:05.00]副歌\n", "lrc").unwrap();
        assert_eq!(doc.segment_count(), 1);
        assert_eq!(doc.segments()[0], "副歌");
        let out = doc.render(&["副歌译文".into()]);
        assert_eq!(out.trim(), "[00:01.00][00:05.00]副歌译文");
    }

    #[test]
    fn lrc_empty_translation_keeps_original() {
        let doc = parse("[00:01.00]原文\n", "lrc").unwrap();
        assert_eq!(doc.render(&[String::new()]).trim(), "[00:01.00]原文");
    }

    #[test]
    fn srt_pairs_each_timestamp_with_its_own_text() {
        // 旧前端按「排序去重后的 cue 下标」取时间段，这里必须各取各的
        let src = "1\n00:00:01,000 --> 00:00:03,000\n甲\n\n2\n00:00:10,000 --> 00:00:12,500\n乙\n";
        let doc = parse(src, "srt").unwrap();
        assert_eq!(doc.segments(), ["甲".to_string(), "乙".to_string()]);
        let out = doc.render(&["甲译".into(), "乙译".into()]);
        assert!(out.contains("00:00:01,000 --> 00:00:03,000\n甲译"), "{out}");
        assert!(out.contains("00:00:10,000 --> 00:00:12,500\n乙译"), "{out}");
    }

    #[test]
    fn vtt_accepts_dot_millis_and_strips_inline_tags() {
        let src = "WEBVTT\n\nNOTE 这是注释\n不该被翻译\n\n00:01.500 --> 00:03.000\n<c.yellow>带标签</c>\n";
        let doc = parse(src, "vtt").unwrap();
        assert_eq!(doc.segments(), ["带标签".to_string()], "NOTE 块整块跳过、内联标签去掉");
        let out = doc.render(&["译文".into()]);
        assert!(out.starts_with("WEBVTT\n"), "{out}");
        assert!(out.contains("00:00:01.500 --> 00:00:03.000\n译文"), "{out}");
    }

    #[test]
    fn time_token_accepts_all_three_shapes() {
        assert_eq!(parse_time_token("00:00:21,813"), Some(21.813));
        assert_eq!(parse_time_token("00:21.813"), Some(21.813));
        assert_eq!(parse_time_token("01:02:03.004"), Some(3723.004));
        assert_eq!(parse_time_token("abc"), None);
    }
}
