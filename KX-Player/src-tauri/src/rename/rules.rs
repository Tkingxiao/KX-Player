//! 改名前置词表与规范化扩展（`04 §7.2` / `04 §7.3.2` / `04 §7.5`）：
//! 1. Unicode NFC 归一（`04 §7.2` 输入行要求）；
//! 2. `04 §7.5` 机翻痕迹（`mt_smell`）的**唯一一份数据**：① 表的词与模式、② 表的启发式；
//! 3. `04 §7.3.2` 长度收敛时的可省修饰语列表。
//!
//! 纯函数、无副作用，守住 norm.rs / judge.rs 的 400 行红线。**判定位只有 `judge::mt_smell` 一处**，
//! 本文件是它的数据源 —— 往黑名单里加词请改这里，别在 `judge.rs` 里再开一张表（第 21 批之前
//! 同一个规则挂着两份数据，一份在 `judge.rs::SMELL_WORDS`、一份在这儿，谁也数不清全不全）。

use unicode_normalization::UnicodeNormalization;

/// 对输入名字执行 Unicode NFC 归一（`04 §7.2` 前置要求）。
/// 解决如日文假名浊点分离（NFD 形式 `か` + `゛` → NFC `が`）等输入异构问题。
pub fn to_nfc(s: &str) -> String {
    s.nfc().collect()
}

/// `04 §7.5` ① 表的「词/模式黑名单」：命中即机翻痕迹。**按表里的行分组写**，
/// 每行至少留一个表里的原例做证据（用例 `spec_row1_examples_are_all_flagged` 逐行核）。
pub const MT_SMELL_WORDS: &[&str] = &[
    // 「癒し」被逐字直译：`理桜的癒的ー試？`
    "的癒", "癒的",
    // 语气词硬译（「哟/哦/呀」出现在陈述位）：`01哟猫鸣馆向。`
    "哟", "吗",
    // 逐字直译出的语义不通组合：`04.理桜和卿卿我我了小姐话哟？`
    "卿卿我我",
    // 主/宾语倒置：`我的酱于啊叫做啊`、`今天也一天疲大人`
    "我的酱", "于啊", "叫做啊", "一天疲",
];

/// `04 §7.5` ① 表「未换中文的日文汉字」那一行：`缶風呂`、`見送`、`揺音`、`特別`、`縦`。
/// 单字一级的「日文新字体 ≠ 简体」由 `judge::TRADITIONAL` 管，这张表收的是**词**
/// （`見送` 里的 `見` 已在 TRADITIONAL，但 `缶風呂` 三个字全都不在 —— 两张表不重叠）。
pub const JAPANESE_KANJI_SMELL: &[&str] = &[
    "缶風呂", "見送", "揺音", "特別", "縦", "目覚",
];

/// `04 §7.5` ②「≥2 个连续单字词」用的虚词表。判的是「两个虚词字贴在一起」——
/// 人写的标题不会这样连，逐字直译才会（`另一侧也掏耳和吗的` 里「和吗」「吗的」两处）。
const PARTICLES: [char; 12] = ['的', '了', '也', '和', '与', '于', '啊', '呀', '哟', '吗', '及', '而'];

/// `04 §7.5` ②「译文出现『的的』『了了』类重复」：这一条**一次就算**，
/// 与上面那条「≥2 处相邻」是两个不同的口径（同字叠出来的一处就已经不通了）。
/// `是是` 是这里比规范多收的一个（原 `judge.rs` 就在判它，并表时照搬，不趁机放宽也不收窄）。
const REPEATED: [&str; 6] = ["的的", "了了", "是是", "也也", "和和", "吗吗"];

/// `04 §7.3.2` 长度超限时，优先省略的可省修饰语（设备名/专有名词如 ASMR、KU100 保留）
pub const OMISSIBLE_MODIFIERS: &[&str] = &[
    "高音质",
    "完全版",
    "限定特典",
    "早期特典",
    "先行版",
    "附赠特典",
];

/// 空括号对（`【】`/`（）`/…）。剥掉修饰语之后留在名字里就是纯噪音，
/// 而括号种类与数量那一关**看不见它坏**（一对空括号仍然「与原文数量一致」）。
/// 八对与 `postcheck::BRACKETS` 同一张清单（`04 §7.3.2` 那一行：`【】『』「」（）()[]`）——
/// 少一对，`【高音质】` 之外的那些就会留下空壳写进名字。
const EMPTY_PAIRS: [(&str, &str); 8] = [
    ("【", "】"), ("『", "』"), ("「", "」"), ("（", "）"), ("(", ")"), ("[", "]"), ("《", "》"), ("〔", "〕"),
];

/// `04 §7.5` ① + ② 的合判：词表、未换汉字的词、虚词重复、连续虚词 ≥2 处、大小写混排的残留英文。
/// ② 的第三条「译文与原文长度比」在这里**故意缺席**，理由写在 `04 §7.5` ② 行末。
pub fn has_mt_smell(s: &str) -> bool {
    MT_SMELL_WORDS.iter().any(|w| s.contains(w))
        || JAPANESE_KANJI_SMELL.iter().any(|w| s.contains(w))
        || REPEATED.iter().any(|w| s.contains(w))
        || adjacent_particles(s) >= 2
        || broken_latin(s)
}

/// 「≥2 个连续单字词」这条启发式的可执行口径：**两个虚词字相邻**算一处，全名要 ≥2 处。
/// 单处不算（`你的名字`、`好了`这种正常标题里就会有一次相邻）。
fn adjacent_particles(s: &str) -> usize {
    let cs: Vec<char> = s.chars().collect();
    cs.windows(2)
        .filter(|w| PARTICLES.contains(&w[0]) && PARTICLES.contains(&w[1]))
        .count()
}

/// 「残留英文拼贴 / 拼写损坏」（§7.5 ① 表最后一行，例 `06_UMenity阿卡贝拉`）：
/// 一段 ≥4 个拉丁字母里**前两个都是大写、第三个起是小写** —— 这是把原文截断拼错留下的形状。
/// 全大写（`ASMR`）与首字母大写（`Love`）、以及带数字的设备名（`KU100`）都不算，避免误伤专名。
fn broken_latin(s: &str) -> bool {
    let mut run: Vec<char> = Vec::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphabetic() {
            run.push(c);
            continue;
        }
        if run.len() >= 4 && run[0].is_uppercase() && run[1].is_uppercase() && run[2..].iter().all(|c| c.is_lowercase()) {
            return true;
        }
        run.clear();
    }
    false
}

/// 尝试先消除可省修饰语以缩减长度（`04 §7.3.2` 长度收敛首选）。
/// 整段被括号包着的修饰语**连括号一起剥**（`【高音质】` → 空壳），否则名字头上会留下一个
/// 看得见的 `【】` —— 它过得了后置第 3 条（数量没变），于是真能写进盘。
pub fn strip_omissible_modifiers(s: &str) -> String {
    let mut res = s.to_string();
    for modifier in OMISSIBLE_MODIFIERS {
        if res.contains(modifier) {
            res = res.replace(modifier, "");
        }
    }
    for (open, close) in EMPTY_PAIRS {
        let pair = format!("{open}{close}");
        loop {
            let (changed, next) = res.replace_once(&pair, "");
            if !changed {
                break;
            }
            res = next;
        }
    }
    res
}

/// 一次替换（`str::replace` 的轻量替身，Windows 上没有 `ReplaceWithFirstMatch`）。
/// 返回 `(是否替换了, 替换后的串)`。
trait ReplaceOnce {
    fn replace_once(&self, from: &str, to: &str) -> (bool, String);
}

impl ReplaceOnce for String {
    fn replace_once(&self, from: &str, to: &str) -> (bool, String) {
        match self.find(from) {
            Some(at) => {
                let mut out = String::with_capacity(self.len());
                out.push_str(&self[..at]);
                out.push_str(to);
                out.push_str(&self[at + from.len()..]);
                (true, out)
            }
            None => (false, self.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename::judge::{judge, Ctx, Reason, Verdict};
    use crate::rename::norm::{truncate, FOLDER_MAX};

    /// 走**判定位**而不是走词表：`04 §7.5` 要的「命中即 NEEDS_REPAIR」是判定树第 10 级的事
    fn smells(name: &str) -> bool {
        let j = judge(name, FOLDER_MAX, &Ctx::default());
        j.reasons.contains(&Reason::MtSmell)
    }

    #[test]
    fn nfc_normalizes_decomposed_kana() {
        // NFD: か (U+304B) + ゛ (U+3099) -> NFC: が (U+304C)
        let nfd = "\u{304B}\u{3099}";
        assert_eq!(nfd.chars().count(), 2);
        let nfc = to_nfc(nfd);
        assert_eq!(nfc.chars().count(), 1);
        assert_eq!(nfc, "が");
    }

    /// `04 §7.5` ① 表**九行、每行的每个例子**各判一次 —— 这张表就是用例的清单，
    /// 往表里加一行就得在这儿加一例，否则那行只是写着好看、实际没人判它。
    /// 唯一例外是第 6 行「中文里夹假名」，它走的是另一档，见下面那条用例。
    #[test]
    fn spec_row1_examples_are_all_flagged() {
        for name in [
            "理桜的癒的ー試？",               // 1 「癒し」逐字直译
            "05癒的耳语",                     // 1 同例，另一个形状
            "另一侧也掏耳和吗的",             // 2 孤立助词收尾（+ 连续虚词两处）
            "一起浴室也进入吗",               // 2 助词留在句尾
            "03.理桜的癒的ー試？",            // 3 语气词硬译（这例同时命中第 1 行）
            "01哟猫鸣馆向。",                 // 3 语气词出现在陈述位
            "04.理桜和卿卿我我了小姐话哟？",   // 4 逐字直译出的不通组合
            "缶風呂",                         // 5 未换中文的日文汉字
            "見送",                           // 5 同上
            "揺音",                           // 5 同上
            "特別",                           // 5 同上
            "縦",                             // 5 同上
            "我的酱于啊叫做啊",               // 7 主/宾语倒置
            "今天也一天疲大人",               // 7 同上，另一个例子
            "06_UMenity阿卡贝拉",             // 8 残留英文拼贴（拼写损坏）
            "part2 – _3",                     // 9 尾部残留分隔符
        ] {
            assert!(smells(name), "§7.5 ① 表的例子没被认出：{name}");
        }
        // 第 9 行的另一个例子 `Track9_` **不在**这张清单里：`_` 是编号自己的分隔符
        // （§7.3.1 前缀逐字节保留），判它重修只会送去一个空片段。反向断言在 `judge.rs` 的用例里。
    }

    /// ②「的的/了了类重复」一次就算；②「≥2 个连续单字词」要两处。两条口径不同，各测一次。
    #[test]
    fn heuristics_use_their_own_thresholds() {
        assert_eq!(adjacent_particles("治愈的的耳边"), 1, "叠字本身只算一处相邻");
        assert!(has_mt_smell("治愈的的耳边"), "②『的的』类重复一次就该命中");
        assert!(!has_mt_smell("好了和"), "②连续虚词只有一处不该命中");
        assert!(has_mt_smell("掏耳和吗的"), "②连续虚词两处该命中");
    }

    /// 第 6 行「中文里夹假名」在判定树里走的是**要翻译**那一档，不是重修：
    /// 假名没翻 = 这条还没译，送去模型译一遍就行，不需要「先译对了再修」。两条都不许放过。
    #[test]
    fn kana_left_in_name_goes_to_translation_not_repair() {
        let j = judge("特典1 迷迭香の時間", FOLDER_MAX, &Ctx::default());
        assert_eq!(j.verdict, Verdict::NeedsTranslation, "夹假名的这条应当判要翻译");
    }

    /// 反例：正常中文标题不许被 ② 那两条启发式误伤
    #[test]
    fn ordinary_titles_stay_clean() {
        for name in ["你的名字", "夏の終わり", "深睡 ASMR 助眠", "KU100 耳朵马杀鸡", "Love 与和平"] {
            assert!(!has_mt_smell(name), "误伤了正常标题：{name}");
        }
    }

    /// ②「≥2 个连续单字词」：一处相邻不算，两处才算
    #[test]
    fn one_adjacent_particle_pair_is_not_enough() {
        assert_eq!(adjacent_particles("好了和"), 1);
        assert!(!has_mt_smell("好了和"));
        assert!(has_mt_smell("掏耳和吗的"));
    }

    #[test]
    fn strips_omissible_modifiers_without_leaving_empty_brackets() {
        // 修饰语被括号整段包着 → 连括号一起走，不许留下 `【】`
        assert_eq!(strip_omissible_modifiers("【高音质】深度睡眠ASMR早期特典"), "深度睡眠ASMR");
        // 括号里还有别的东西 → 括号留着，只挖掉修饰语
        assert_eq!(strip_omissible_modifiers("【高音质 人声】夜色"), "【 人声】夜色");
        // 半角方括号也在规范的清单里（`04 §7.3.2`：`【】『』「」（）()[]`），少一对就漏掉空壳
        assert_eq!(strip_omissible_modifiers("[高音质]名称"), "名称");
    }

    /// `truncate` 的三条分支各一条：剥完刚好不超 / 仍超要截 / 一个修饰语都没命中
    #[test]
    fn truncate_branches_are_all_exercised() {
        let fits_after_strip = format!("【高音质】{}", "夜".repeat(118));
        assert_eq!(fits_after_strip.chars().count(), 123, "夹具得先真的超长");
        let out = truncate(&fits_after_strip, FOLDER_MAX);
        assert_eq!(out.chars().count(), 118, "剥完就该不超长了，不该再补省略号");
        assert!(out.starts_with('夜'), "括号壳没跟着走：{out}");
        let still_too_long = truncate(&"长".repeat(FOLDER_MAX + 30), FOLDER_MAX);
        assert!(still_too_long.ends_with('…') && still_too_long.chars().count() == FOLDER_MAX);
        let untouched = truncate("纯中文没有修饰语", FOLDER_MAX);
        assert_eq!(untouched, "纯中文没有修饰语", "没超长就不该动它");
    }
}
