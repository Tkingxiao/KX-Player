//! 改名的**本地规范化**规则（`04 §7.3`）：先把序号前缀切出来原样留着，剩下的文本再按符号表收敛。
//!
//! 纯函数、零 IO —— 这里只回答「规范化之后长什么样」。要不要动由 `judge.rs` 判，动不动得动由
//! 调用方（预览 / 应用）决定。规则里凡是「保留」的都比「统一」优先：用户的编号方式
//! （`hi09_06_` / `01` / `【01】`）本身就是信息，重排会把参考库里的对应关系打乱（`04 §7.3.1`）。
//! 能在规则里消化掉的不留给模型（`04 §7.11` 的成本红线）—— 大量条目只是波浪号不统一、结尾多个空格。

/// 文件夹名长度上限（`04 §7.2` 第 7 条）。计数单位是字符不是字节，中文不按 3 字节算。
pub const FOLDER_MAX: usize = 120;
/// 文件名主体长度上限（不含扩展名）。
pub const FILE_MAX: usize = 100;
/// 全路径长度上限，给 Windows 的 260 留余量。
pub const PATH_MAX: usize = 240;

/// 名字开头那段编号 + 它之后的正文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// 命中部分，逐字节原样（`04 §7.3.1`）；没命中就是空串
    pub prefix: &'a str,
    /// 需要继续处理／翻译的剩余文本
    pub rest: &'a str,
}

/// 编号属于哪一族（`04 §7.3.1` 的四族）。同目录里出现 >1 族就是 `prefix_mixed`，只报告不自动统一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// 没有编号前缀
    None,
    /// `【01】`
    Bracket,
    /// `hi09_06_` `tr00_` `track01_` `音轨1` `附赠01` `特典1` `day00_`
    Keyword,
    /// `01_` `01.` `1-` `01`（后跟正文的特例）
    Digits,
}

/// 按 `04 §7.3.1` 切序号前缀：`【NNN】` → 关键字族 → 数字+分隔符 → 数字紧跟正文的特例。
/// 切不出来的前缀一律算正文（`倒计时语音_1天前` 这种中缀序号不在规则范围内，不猜）。
pub fn split_prefix(name: &str) -> Split<'_> {
    let cs: Vec<char> = name.chars().collect();
    let n = bracket_len(&cs)
        .or_else(|| keyword_len(&cs))
        .or_else(|| digits_sep_len(&cs))
        .or_else(|| digits_bare_len(&cs))
        .unwrap_or(0);
    // 前缀可能整段都是 ASCII 也可能混着中文，按字符数还原字节偏移
    let cut = cs[..n].iter().map(|c| c.len_utf8()).sum();
    Split {
        prefix: &name[..cut],
        rest: &name[cut..],
    }
}

/// 前缀归族。传 `split_prefix` 出来的 `prefix` 即可。
pub fn family(prefix: &str) -> Family {
    match prefix.chars().next() {
        None => Family::None,
        Some('【') => Family::Bracket,
        Some(c) if c.is_ascii_digit() => Family::Digits,
        Some(_) => Family::Keyword,
    }
}

/// 规范化一个名字（不含扩展名：扩展名永不参与，`04 §7.2` 第 2 条）。
///
/// 前缀只做空白折叠，符号与截断都作用在「前缀 + 处理后的正文」上。整段就是编号的名字
/// （`Track1.`）原样返回 —— 编号是它的全部内容，动它就等于把文件改成没名字。
pub fn normalize(name: &str, max: usize) -> String {
    let nfc = crate::rename::rules::to_nfc(name);
    let sp = split_prefix(&nfc);
    if sp.rest.trim().is_empty() {
        return truncate(nfc.trim(), max);
    }
    let mut out = String::with_capacity(nfc.len());
    out.push_str(&fold_ws(sp.prefix));
    out.push_str(&clean_text(sp.rest));
    truncate(&out, max)
}

/// 超长收敛（`04 §7.3.2` 最后一行）：先省可省修饰语，仍在词边界截断并加 `…`，不切半个词出来。
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let stripped = crate::rename::rules::strip_omissible_modifiers(s);
    // 剥修饰语会把**它占过的位置**留在原地：`高音质 完全版 名称` 剥完是 `  名称`（首空格 + 连续空格），
    // `名称 高音质` 剥完是 `名称 `（尾空格）。这些正是判定树第 4 条要修的形态，收不干净就等于
    // 把一条「该重修」的名字写进盘，下一轮预览又判它一遍。收法与 [`clean_text`] 同出一处（[`tidy_ws`]）。
    let tidy = tidy_ws(&stripped);
    if !tidy.is_empty() && tidy.chars().count() <= max {
        return tidy;
    }
    // 剥到什么都不剩（整名就是修饰语叠出来的）时不能采用这个空结果 —— 文件名不能被剥没了
    let target = if tidy.is_empty() { s } else { tidy.as_str() };
    let keep = max - 1; // 留给省略号
    let mut head: String = target.chars().take(keep).collect();
    if let Some(at) = head.rfind([' ', '_', '・', '、']) {
        if head[..at].chars().count() > keep / 2 { head.truncate(at); }
    }
    head.push('…');
    head
}

/// 符号映射表（`04 §7.3.2`）。括号一类**不碰**：那条规则要求「种类与数量不变」，
/// 半角转全角改的就是种类（`04 §7.10` 第三例把这件事记成 notes 而不是失败）。
pub fn map_char(c: char) -> char {
    match c {
        '〜' | '~' | '∼' | '〰' => '～',
        '!' => '！',
        '?' => '？',
        '\u{3000}' => ' ',
        _ => c,
    }
}

fn clean_text(s: &str) -> String {
    let mapped: String = s.chars().map(map_char).collect();
    tidy_ws(&fold_dots(&fix_times(&mapped)))
}

/// 空白与尾部符号的**唯一一份**收尾口径：去首尾空白 → 全角空格转半角、连续空格收成一个 →
/// 去尾部 ` ` `.` `_`。本地规范化（[`clean_text`]、[`truncate`]）与后置第 8 条（`postcheck`）
/// 用的都是这一份 —— 名字从哪条路走过来都得落到同一个结果，各写各的迟早会分叉。
pub(crate) fn tidy_ws(s: &str) -> String {
    fold_ws(s.trim()).trim_end_matches([' ', '.', '_']).to_string()
}

/// 时间表达（`04 §7.3.2`）：数字一个都不许变，只换单位字。`30分` 后面已经是「钟」时不能重复加。
fn fix_times(s: &str) -> String {
    let s = s.replace("時間", "小时").replace("分鐘", "分钟");
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    for (i, c) in cs.iter().enumerate() {
        out.push(*c);
        if *c == '分' && i > 0 && cs[i - 1].is_ascii_digit() && cs.get(i + 1) != Some(&'钟') {
            out.push('钟');
        }
    }
    out
}

/// 连续 `.` 压成 `.`（`04 §7.3.2` 结尾清理那行）。省略号 `…` 是另一个字符，不参与。
fn fold_dots(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dot = false;
    for c in s.chars() {
        if c == '.' {
            if !prev_dot {
                out.push('.')
            }
        } else {
            out.push(c)
        }
        prev_dot = c == '.';
    }
    out
}

/// 全角空格→半角，连续空格→一个。前缀内部也要折叠，否则 `01  名称` 既命中 `whitespace`
/// 又永远规范化不出变化，判定和结果自相矛盾。`postcheck` 的第 8 条也用它 —— `§7.7` 给的阈值是
/// 「3 个以上连续空格」，直接收成单空格比它更严，好处是同一条名字走本地规则与走后置校验得到同一个结果。
pub(crate) fn fold_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for c in s.chars() {
        let c = if c == '\u{3000}' { ' ' } else { c };
        if c == ' ' {
            if !prev_ws {
                out.push(' ')
            }
            prev_ws = true;
        } else {
            prev_ws = false;
            out.push(c);
        }
    }
    out
}

/// `【01】`：一个到三个数字，两边方括号照原样留
fn bracket_len(cs: &[char]) -> Option<usize> {
    if cs.first() != Some(&'【') {
        return None;
    }
    let d = digit_run(&cs[1..], 3)?;
    if cs.get(1 + d) == Some(&'】') {
        Some(2 + d)
    } else {
        None
    }
}

/// 关键字族：`附赠音轨|附赠|特典|音轨|track|tr|hiNN|day` + 序号 + 尾随分隔符。
/// `track` 必须排在 `tr` 前面，否则 `track01` 会被切成 `tr` 加一段认不出的正文。
const KEYWORDS: [&str; 7] = ["附赠音轨", "音轨", "附赠", "特典", "track", "tr", "day"];

fn keyword_len(cs: &[char]) -> Option<usize> {
    let mut head = 0;
    if starts_hi(cs) {
        head = 4;
    } else {
        for kw in KEYWORDS {
            if matches_kw(cs, kw) {
                head = kw.chars().count();
                break;
            }
        }
    }
    if head == 0 {
        return None;
    }
    let tail = seq_tail(cs, head)?;
    Some(head + tail)
}

/// 关键字之后那一段：`[_\-\s]*` + 一到四个数字 + `[_\-.、\s]*`。缺数字就不算关键字族。
fn seq_tail(cs: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while matches!(cs.get(i), Some(&'_') | Some(&'-') | Some(&' ') | Some(&'\u{3000}')) {
        i += 1;
    }
    let d = digit_run(&cs[i..], 4)?;
    i += d;
    while matches!(
        cs.get(i),
        Some(&'_') | Some(&'-') | Some(&'.') | Some(&'、') | Some(&' ') | Some(&'\u{3000}')
    ) {
        i += 1;
    }
    Some(i - from)
}

fn matches_kw(cs: &[char], kw: &str) -> bool {
    kw.chars().enumerate().all(|(i, k)| {
        cs.get(i)
            .is_some_and(|c| c.to_ascii_lowercase() == k)
    })
}

/// `hi09` —— `hi` 只有紧跟两位数字时才算关键字（`hi` 开头的正文不Cut）
fn starts_hi(cs: &[char]) -> bool {
    cs.first().is_some_and(|c| matches!(c, 'h' | 'H'))
        && cs.get(1).is_some_and(|c| matches!(c, 'i' | 'I'))
        && cs.get(2).is_some_and(|c| c.is_ascii_digit())
        && cs.get(3).is_some_and(|c| c.is_ascii_digit())
}

/// `01_` `1-` `01.` `01、`：一到三个数字后必须紧跟分隔符
fn digits_sep_len(cs: &[char]) -> Option<usize> {
    let d = digit_run(cs, 3)?;
    let mut i = d;
    while matches!(
        cs.get(i),
        Some(&'_') | Some(&'-') | Some(&'.') | Some(&'、') | Some(&' ') | Some(&'\u{3000}')
    ) {
        i += 1;
    }
    if i == d {
        return None;
    }
    Some(i)
}

/// 特例（`04 §7.3.1`）：`01第一回款待皇家。` 前缀后没有分隔符也要切得开。
/// 只认两位数字，且紧跟时间/量词时不切 —— `16小时`、`2時間`、`30分` 是正文不是编号。
const NOT_A_PREFIX: [char; 9] = ['分', '秒', '时', '小', '年', '月', '日', '天', '時'];

fn digits_bare_len(cs: &[char]) -> Option<usize> {
    if cs.len() < 3 || !cs[0].is_ascii_digit() || !cs[1].is_ascii_digit() {
        return None;
    }
    let next = cs[2];
    if next.is_ascii() || NOT_A_PREFIX.contains(&next) {
        return None;
    }
    Some(2)
}

/// 开头连续的 ASCII 数字，最多取 `max` 个；一个都没有返回 None
fn digit_run(cs: &[char], max: usize) -> Option<usize> {
    let mut n = 0;
    while n < max && cs.get(n).is_some_and(|c| c.is_ascii_digit()) {
        n += 1;
    }
    if n == 0 {
        None
    } else {
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_families_from_the_reference_library_all_split() {
        // 04 §7.3.1 列出的那一排，逐个都得认出来（这些形状就是用户既有的约定）
        for (name, prefix) in [
            ("hi09_06_名称", "hi09_06_"),
            ("hi08_12_名称", "hi08_12_"),
            ("tr00_名称", "tr00_"),
            ("tr_1名称", "tr_1"),
            ("track00_名称", "track00_"),
            ("Track1.名称", "Track1."),
            ("音轨0_名称", "音轨0_"),
            ("音轨1名称", "音轨1"),
            ("附赠01名称", "附赠01"),
            ("附赠音轨1名称", "附赠音轨1"),
            ("特典1名称", "特典1"),
            ("day00_名称", "day00_"),
            ("【01】名称", "【01】"),
            ("01_名称", "01_"),
            ("01.名称", "01."),
            ("1-名称", "1-"),
            ("01、名称", "01、"),
        ] {
            assert_eq!(split_prefix(name).prefix, prefix, "{name}");
        }
    }

    #[test]
    fn prefix_without_separator_splits_but_time_expressions_do_not() {
        assert_eq!(split_prefix("01第一回款待皇家。").prefix, "01");
        for name in ["16小时", "2時間30分以上", "30分钟", "12月的记忆"] {
            assert_eq!(split_prefix(name).prefix, "", "{name} 不该被当前缀");
        }
    }

    #[test]
    fn prefix_is_verbatim_and_body_is_what_is_left() {
        let sp = split_prefix("05_UMenity阿卡贝拉");
        assert_eq!((sp.prefix, sp.rest), ("05_", "UMenity阿卡贝拉"));
        assert_eq!(split_prefix("倒计时语音_1天前").prefix, "", "中缀序号不在规则范围内");
        assert_eq!(split_prefix("普通名字").prefix, "");
    }

    #[test]
    fn families_separate_so_a_mixed_directory_shows_up() {
        assert_eq!(family("【01】"), Family::Bracket);
        assert_eq!(family("hi09_06_"), Family::Keyword);
        assert_eq!(family("音轨1"), Family::Keyword);
        assert_eq!(family("01_"), Family::Digits);
        assert_eq!(family(""), Family::None);
    }

    #[test]
    fn wave_and_halffull_are_unified() {
        assert_eq!(normalize("测试～名称", FOLDER_MAX), "测试～名称");
        assert_eq!(normalize("测试〜名称", FOLDER_MAX), "测试～名称");
        assert_eq!(normalize("测试~名称", FOLDER_MAX), "测试～名称");
        assert_eq!(normalize("快来!!", FILE_MAX), "快来！！");
        // 括号种类不变：半角进去还是半角（04 §7.3.2 那行「种类与数量不变」）
        assert_eq!(normalize("(测试)【名】", FILE_MAX), "(测试)【名】");
    }

    #[test]
    fn underscores_and_medium_dots_survive_untouched() {
        assert_eq!(normalize("掏耳・吐息_副标题", FILE_MAX), "掏耳・吐息_副标题");
    }

    #[test]
    fn whitespace_is_folded_around_the_prefix_too() {
        assert_eq!(normalize("01  名称", FOLDER_MAX), "01 名称");
        assert_eq!(normalize("特典1　迷迭香", FOLDER_MAX), "特典1 迷迭香");
        assert_eq!(normalize("  名称  ", FILE_MAX), "名称");
    }

    #[test]
    fn tail_junk_is_stripped_but_prefix_dots_stay() {
        assert_eq!(normalize("01.名称._ ", FILE_MAX), "01.名称");
        assert_eq!(normalize("名称...", FILE_MAX), "名称");
        assert_eq!(normalize("Track1.", FILE_MAX), "Track1.", "整段就是编号 → 不动");
        assert_eq!(normalize("01_名称。", FILE_MAX), "01_名称。", "句号是全角的，不在清理名单");
    }

    #[test]
    fn time_expressions_rewritten_without_touching_digits() {
        assert_eq!(normalize("16時間", FILE_MAX), "16小时");
        assert_eq!(normalize("2時間30分以上", FILE_MAX), "2小时30分钟以上");
        assert_eq!(normalize("30分钟", FILE_MAX), "30分钟", "已经是分钟的不重复加");
    }

    #[test]
    fn over_length_cuts_at_a_word_boundary_and_marks_it() {
        assert_eq!(truncate(&"呀".repeat(130), 120).chars().count(), 120);
        // 有空格可退就退到空格前，别把「六七八」这种截半的词留在结尾
        assert_eq!(truncate("一二三四 六七八九十 十一", 8), "一二三四…");
        assert_eq!(truncate("一二三", 8), "一二三", "没超长原样返回");
        let long = format!("{}尾巴", "呀".repeat(130));
        let out = normalize(&long, FOLDER_MAX);
        assert!(out.ends_with('…') && out.chars().count() == FOLDER_MAX);
    }

    /// 剥修饰语会把**它占过的位置**留在原地：`高音质 完全版 名称` 剥完是 `  名称`、`名称 高音质`
    /// 剥完尾巴上多个空格。那些正是判定树第 4 条要修的形态，收不干净就是把一条「该重修」的名字
    /// 写进盘（尾空格更坏：Windows 会把它悄悄去掉，history 里那份映射就对不上盘上的名字）。
    #[test]
    fn truncate_leaves_no_dirty_whitespace_after_stripping() {
        let tail = "夜".repeat(118); // 剥完刚好 120
        let out = truncate(&format!("高音质 完全版 {tail}"), FOLDER_MAX);
        assert_eq!(out, tail, "剥完的空档没收干净：{out}");

        let out = truncate(&format!("{tail}名 高音质"), FOLDER_MAX);
        assert_eq!(out, format!("{tail}名"), "修饰语在末尾时尾巴会多个空格：{out}");

        // 收干净的标准是「再过一遍规范化不再变」：改完就定稿
        for name in ["高音质 完全版 名称", "名称 高音质", "名称.高音质"] {
            let long = format!("{name}{}", "夜".repeat(120));
            let once = normalize(&long, FOLDER_MAX);
            assert_eq!(normalize(&once, FOLDER_MAX), once, "{name} 改完还要再改一遍");
        }
    }

    /// 整名就是修饰语叠出来的：剥到一个字不剩时不许把名字剥没（文件不能没有名字）
    #[test]
    fn stripping_everything_falls_back_to_a_truncated_name() {
        let all_mod = "高音质".repeat(41);
        assert!(all_mod.chars().count() > FOLDER_MAX, "夹具得先真的超长");
        let out = truncate(&all_mod, FOLDER_MAX);
        assert!(!out.is_empty(), "剥空了就得退回截断，而不是交一个空名字");
        assert!(out.ends_with('…') && out.chars().count() == FOLDER_MAX);
    }

    #[test]
    fn normalize_is_idempotent() {
        for name in [
            "hi09_06_测试名称", "01  名称~~", "特典1　2時間30分以上", "【01】快来!!",
        ] {
            let once = normalize(name, FOLDER_MAX);
            assert_eq!(normalize(&once, FOLDER_MAX), once, "{name} 二次规范化又变了");
        }
        // NFD: か (U+304B) + ゛ (U+3099)
        assert!(normalize("\u{304B}\u{3099}音01", FOLDER_MAX).starts_with("が"));
    }
}
