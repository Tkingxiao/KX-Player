//! AI 之后必过的九道**后置校验**（`04 §7.7`）：模型给的名字不过这一关就不能写盘。
//!
//! 纯函数 —— 校验的对象是「一段文本和它的原文」，既不需要文件系统也不需要数据库。九道按
//! 规范表里的编号实现，但**第 9 条最先判**：用户拒过的建议拿去修好再写盘，等于跟用户对着干。
//! 两类失败的处理方式不同：能本地修的（1/2/5/6/8 条）当场修并记 `notes`，修不得的硬伤
//! （3/4/7 条：括号、数字、假名没翻成中文）只能退回重试，重试过还有硬伤就交给人。
//!
//! 校验范围是**送进模型的那段正文**：序号前缀与扩展名由调用方剥掉后再拼回（`04 §7.6` 的设计
//! 前提是「送进模型的只有真正需要翻译的那几个字」），所以 `max_len` 是**正文预算**，不含前缀。

use crate::rename::judge::{is_han, is_kana};
use crate::rename::norm;

/// 一条模型输出过完九道校验之后的结论
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 原样能用
    Accepted,
    /// 本地修过之后能用（`notes` 里是修了哪几处）
    Fixed,
    /// 有硬伤：带错误说明重试一次
    Retry,
    /// 重试过了仍有硬伤：这条不写库，摆给人看
    Review,
    /// 用户拒过这条建议（`04 §7.7` 第 9 条），直接丢弃
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub name: String,
    pub action: Action,
    /// 本地**修了什么**（前缀、扩展名、非法字符、长度、空白）—— 落库时随译文存着
    pub notes: Vec<String>,
    /// 卡在哪一关（§7.7 的 3/4/7/9 条）。`Retry`/`Review`/`Rejected` 时非空，是给用户看的理由
    pub blocked: Vec<String>,
}

/// 校验一条建议要的外部事实
#[derive(Debug, Clone)]
pub struct Ctx<'a> {
    /// 送给模型的那段原文（前缀与扩展名都已剥掉）
    pub original: &'a str,
    /// 正文长度上限
    pub max_len: usize,
    /// 这个条目上用户拒过的候选名（`rejected_suggestions`）
    pub refused: &'a [String],
    /// 已经是重试回来的那一轮
    pub retried: bool,
}

/// 括号一类：`04 §7.3.2` 明令本地规则不碰它们，§7.6.1 第 2 条又要求模型别写括号注释，
/// 所以这里只核「种类与数量一个都不许变」。
const BRACKETS: [char; 14] = ['(', ')', '（', '）', '【', '】', '「', '」', '『', '』', '《', '》', '〔', '〕'];
/// Windows 真正不让出现在文件名里的字符。`／⧸` 是形近字、不算在这里 —— 它们写得进盘，
/// 而 `04 §7.3.2` 要求保留现状。
const FORBID: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

pub fn check(dst: &str, ctx: &Ctx<'_>) -> Checked {
    let trimmed = dst.trim();
    if ctx.refused.iter().any(|r| r == trimmed) {
        return Checked {
            name: trimmed.to_string(),
            action: Action::Rejected,
            notes: Vec::new(),
            blocked: vec!["这条建议用户拒过，不再提".to_string()],
        };
    }

    let mut s = dst.to_string();
    let mut notes: Vec<String> = Vec::new();
    // 硬伤：本地修不了，只能退回重试（§7.7 的 3/4/7 条）
    let mut blocked: Vec<String> = Vec::new();

    // 1 序号前缀逐字符一致 —— 模型改前缀是常见且无害的错误，覆盖回来就行，不重试
    let src_prefix = norm::split_prefix(ctx.original).prefix;
    let dst_prefix = norm::split_prefix(&s).prefix.to_string();
    if src_prefix != dst_prefix.as_str() {
        s = format!("{src_prefix}{}", &s[dst_prefix.len()..]);
        notes.push(format!("序号前缀改回原文：{dst_prefix} → {src_prefix}"));
    }

    // 2 扩展名不变：契约里片段根本不带扩展名，所以这条实际拦的是「模型自己补了一个」
    let (src_ext, dst_ext) = (ext_of(ctx.original), ext_of(&s));
    if src_ext != dst_ext {
        if let Some(e) = dst_ext {
            s.truncate(s.len() - e.len());
        }
        s.push_str(src_ext.unwrap_or(""));
        notes.push("扩展名改回原文".to_string());
    }

    // 3 括号种类与数量 / 4 数字多重集
    if !same_counts(ctx.original, &s, &BRACKETS) {
        blocked.push("括号的种类或数量与原文不符".to_string());
    }
    if digits(ctx.original) != digits(&s) {
        blocked.push("数字与原文对不上（01 不许写成 1）".to_string());
    }

    // 5 无新增非法字符：`/` 与 `\` 换成 `⧸`，其余删掉 —— 带着它们根本写不进盘
    s = sanitize(&s, ctx.original, &mut notes);

    // 6 长度合规（词边界截断 + `…`）
    let len = s.chars().count();
    if len > ctx.max_len {
        s = norm::truncate(&s, ctx.max_len);
        notes.push(format!("超长已截断：{len} → {}", s.chars().count()));
    }

    // 7 原文含假名则译文必须含中文
    if ctx.original.chars().any(is_kana) && !s.chars().any(is_han) {
        blocked.push("原文有假名而译文里没有中文".to_string());
    }

    // 8 连续空格 / 首尾空白 / 尾部句点：一律本地清掉，不劳模型
    let cleaned = norm::fold_ws(s.trim()).trim_end_matches([' ', '.', '_']).to_string();
    if cleaned != s {
        notes.push("空白与尾部符号已清理".to_string());
    }
    s = cleaned;

    // 硬伤优先于「修好了」：能本地修的都修了，剩下的那些决定这一条算不算可用
    let action = match (blocked.is_empty(), ctx.retried) {
        (true, _) if s == trimmed => Action::Accepted,
        (true, _) => Action::Fixed,
        (false, false) => Action::Retry,
        (false, true) => Action::Review,
    };
    Checked { name: s, action, notes, blocked }
}

/// 卡住的原因拼成一行，给重试请求当「错误说明」（`04 §7.7` 第 3、4 条要求带说明退回）。
pub fn retry_hint(blocked: &[String]) -> String {
    format!("上一次的答案不合格：{}。请只改这一处，其余照旧。", blocked.join("；"))
}

/// 尾部扩展名（`.mp3`）。点号后必须是 1~6 个 ASCII 字母数字才算，否则 `3.3`、`Track1.`
/// 这类名字里的句点会被当成扩展名。
fn ext_of(s: &str) -> Option<&str> {
    let at = s.rfind('.')?;
    let tail = &s[at + 1..];
    (at > 0 && !tail.is_empty() && tail.len() <= 6 && tail.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| &s[at..])
}

fn same_counts(a: &str, b: &str, set: &[char]) -> bool {
    set.iter().all(|c| a.chars().filter(|x| x == c).count() == b.chars().filter(|x| x == c).count())
}

/// 数字按**连续段**取，再排序比较（`04 §7.7` 第 4 条的多重集）。
/// `16:00` 与 `00:16` 的段集合相同，所以这条只拦「数字被改掉」，不拦「顺序被调过」。
fn digits(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            run.push(c);
        } else if !run.is_empty() {
            out.push(std::mem::take(&mut run));
        }
    }
    out.sort();
    out
}

/// 清掉写不进盘的字。规范说的是「无**新增**非法字符」，但原文里本来就有的留着也一样落不了地，
/// 所以一律处理；只有模型自己添的那些记 `notes`（§7.7 末行要求任何一项失败都留可见记录）。
fn sanitize(s: &str, original: &str, notes: &mut Vec<String>) -> String {
    let src: Vec<char> = original.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut hit: Vec<char> = Vec::new();
    for c in s.chars() {
        if FORBID.contains(&c) {
            if !hit.contains(&c) {
                hit.push(c);
            }
            if c == '\\' || c == '/' {
                out.push('\u{29F8}');
            }
            continue;
        }
        out.push(c);
    }
    let fresh: Vec<String> =
        hit.iter().filter(|c| !src.contains(c)).map(|c| c.to_string()).collect();
    if !fresh.is_empty() {
        notes.push(format!("非法字符已处理：{}", fresh.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{check, Ctx};

    fn ctx<'a>(original: &'a str, max_len: usize, refused: &'a [String], retried: bool) -> Ctx<'a> {
        Ctx { original, max_len, refused, retried }
    }

    fn plain<'a>(original: &'a str) -> Ctx<'a> {
        ctx(original, 100, &[], false)
    }

    /// ① 序号前缀逐字符覆盖回来，且**不算重试理由**（模型改前缀无害）
    #[test]
    fn step1_forces_the_original_prefix_back() {
        let c = plain("静かな夜");
        let r = super::check("01_宁静的夜", &c);
        assert_eq!(r.name, "宁静的夜");
        assert_eq!(r.action, super::Action::Fixed);
        assert!(r.notes[0].contains("01_"), "{:?}", r.notes);
    }

    /// ② 片段里没有扩展名，模型补一个就得剥掉
    #[test]
    fn step2_strips_an_invented_extension() {
        let r = check("治愈之音.mp3", &plain("癒しの音"));
        assert_eq!(r.name, "治愈之音");
    }

    /// ③ 括号换种类是硬伤：先 Retry，重试回来还不行就 Review
    #[test]
    fn step3_bracket_swap_retries_then_reviews() {
        let first = check("回忆(日常)", &plain("思い出（日常）"));
        assert_eq!(first.action, super::Action::Retry);
        let second = check("回忆(日常)", &ctx("思い出（日常）", 100, &[], true));
        assert_eq!(second.action, super::Action::Review);
        assert!(second.blocked.iter().any(|b| b.contains("括号")), "{:?}", second.blocked);
    }

    /// ④ 数字多重集：换成汉字数字要退回去，`16時間`→`16小时` 这种保留数字的放行
    #[test]
    fn step4_digits_must_survive_translation() {
        assert_eq!(check("16小时", &plain("16時間")).name, "16小时");
        let r = check("十六小时", &plain("16時間"));
        assert_eq!(r.action, super::Action::Retry);
        assert!(r.blocked.iter().any(|b| b.contains("数字")), "{:?}", r.blocked);
    }

    /// ⑤ 斜杠换成形近的 `⧸`，其余非法字符删掉；原文里本来就有的不记 notes
    #[test]
    fn step5_removes_what_windows_cannot_write() {
        assert_eq!(check("A:B*C?D", &plain("名前")).name, "ABCD");
        assert_eq!(check("A/B", &plain("名前")).name, "A\u{29F8}B");
        let kept = check("A/B", &plain("A/B"));
        assert_eq!(kept.name, "A\u{29F8}B");
        assert!(kept.notes.is_empty(), "原文里就有的字符不该记成模型的错：{:?}", kept.notes);
    }

    /// ⑥ 超限在词边界截断并留 `…`
    #[test]
    fn step6_truncates_over_long_names() {
        let long = "とても長い名前".repeat(4);
        let r = check(&long, &ctx("名前", 8, &[], false));
        assert_eq!(r.name.chars().count(), 8);
        assert!(r.name.ends_with('…'));
        assert!(r.notes.iter().any(|n| n.contains("截断")), "{:?}", r.notes);
    }

    /// ⑦ 原文有假名而译文没中文 = 压根没翻，属于硬伤
    #[test]
    fn step7_kana_input_must_come_back_as_chinese() {
        assert_eq!(check("治愈之音", &plain("癒しの音")).action, super::Action::Accepted);
        assert_eq!(check("healing sound", &plain("癒しの音")).action, super::Action::Retry);
    }

    /// ⑧ 连续空格、首尾空白、尾部句点本地清掉，不劳模型
    #[test]
    fn step8_cleans_whitespace_and_tails() {
        let r = check("  名字   后缀.  ", &plain("名前 サフィックス"));
        assert_eq!(r.name, "名字 后缀");
    }

    /// ⑨ 用户拒过的建议**先**被拦下：后面九步都不该为它跑
    #[test]
    fn step9_refused_suggestions_are_dropped_before_repair() {
        let refused = vec!["治愈 之音".to_string()];
        let r = check("  治愈 之音  ", &ctx("癒しの音", 100, &refused, false));
        assert_eq!(r.action, super::Action::Rejected);
        assert!(r.name.starts_with("治愈"), "只该 trim，不该接着修：{}", r.name);
        assert!(!r.name.ends_with(' '), "落库比对用的是去空白后的名字");
    }
}
