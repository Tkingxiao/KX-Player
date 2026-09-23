//! **前置判定树**（`04 §7.2`）：纯本地、零成本、可离线，决定「要不要动、动到什么程度」。
//!
//! 十四级从上往下第一个命中即定档，`SKIP` 最高、`NEEDS_REPAIR` 高于 `NEEDS_TRANSLATION`。
//! 排序的全部理由是把纯字符问题拦在模型之前：这类项一秒钟都不该花在模型上（`04 §7.2`）。
//! 命中的原因**全部**收集进 `reasons`（`04 §7.10` 第一例要同时看到 `kana` 和 `illegal_char`），
//! 定档只看第一个「有动作」的原因。

use crate::rename::norm::{self, Family};

/// 动到什么程度
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 不碰
    Skip,
    /// 只跑本地规则，不进 AI
    NormalizeOnly,
    /// 旧译有机翻痕迹，要重修（旧译当负样本传，`04 §7.5`）
    NeedsRepair,
    /// 需要翻译
    NeedsTranslation,
    /// 已经合规
    Compliant,
}

/// 命中的那一条规则（`04 §7.2` 的编号顺序）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// 用户锁定
    Locked,
    /// 用户拒绝过这条建议
    Rejected,
    /// 非法/形近字符。`04 §7.3.2` 规定 `／⧸\` 保留现状，所以它只记不定档
    IllegalChar,
    /// 全角空格 / 连续空格 / 首尾空格 / 结尾句点
    Whitespace,
    /// 波浪号混用
    Wave,
    /// 半角全角混用
    HalfFull,
    /// 长度超限
    OverlyLong,
    /// 同目录序号族不统一（只报告，不自动改前缀）
    PrefixMixed,
    /// 文件名里重复出现所属文件夹名
    DuplicateTitle,
    /// 机翻痕迹
    MtSmell,
    /// 含假名
    Kana,
    /// 含繁体或残留日文汉字
    Traditional,
    /// 纯 ASCII 罗马字
    Roman,
}

/// 判定结果：档位 + 全部命中原因（按 `04 §7.2` 的顺序）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgment {
    pub verdict: Verdict,
    pub reasons: Vec<Reason>,
}

/// 已被用户锁或拒。变体在本批**没有构造方** —— 锁定/排除是 S2 的用户操作，磁盘上没有对应事实：
/// `Rejected` 的落点规范给了表（`04 §5:299` `rejected_suggestions`，按 `target_path` 记被拒的候选名），
/// `Locked` 规范没给（要么落成 `rename_items` 的行状态、要么加一列，等 S2 定）。
/// 判定树是 `04 §7.2` 的契约，未构造 ≠ 可删（同 `error.rs` 的码表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Skipped {
    Locked,
    Rejected,
}

/// 单个名字判不出来的那三条要外部给（`04 §7.2` 第 1、8、9 条）
#[derive(Debug, Default, Clone, Copy)]
pub struct Ctx<'a> {
    /// 锁定 / 已拒绝
    pub skipped: Option<Skipped>,
    /// 所属文件夹名：文件名里整段重复它 → `duplicate_title`
    pub folder: Option<&'a str>,
    /// 同目录内序号前缀族 >1 种，由扫描层用 `norm::family` 统计
    pub prefix_mixed: bool,
}

/// 判定一个名字（不含扩展名 —— 扩展名永不参与，`04 §7.2` 第 2 条）。
/// `max` 是这一类名字的长度上限（`FOLDER_MAX` / `FILE_MAX`）。
pub fn judge(name: &str, max: usize, ctx: &Ctx<'_>) -> Judgment {
    let mut reasons: Vec<Reason> = Vec::new();
    if let Some(s) = ctx.skipped {
        reasons.push(match s {
            Skipped::Locked => Reason::Locked,
            Skipped::Rejected => Reason::Rejected,
        });
    }
    if illegal(name) {
        reasons.push(Reason::IllegalChar);
    }
    if whitespace(name) {
        reasons.push(Reason::Whitespace);
    }
    if wave(name) {
        reasons.push(Reason::Wave);
    }
    if half_full(name) {
        reasons.push(Reason::HalfFull);
    }
    if name.chars().count() > max {
        reasons.push(Reason::OverlyLong);
    }
    if ctx.prefix_mixed && norm::family(norm::split_prefix(name).prefix) != Family::None {
        reasons.push(Reason::PrefixMixed);
    }
    if ctx
        .folder
        .is_some_and(|f| f.chars().count() >= 2 && !name.is_empty() && name.contains(f))
    {
        reasons.push(Reason::DuplicateTitle);
    }
    if mt_smell(name) {
        reasons.push(Reason::MtSmell);
    }
    if name.chars().any(is_kana) {
        reasons.push(Reason::Kana);
    }
    if traditional(name) {
        reasons.push(Reason::Traditional);
    }
    if roman(name) {
        reasons.push(Reason::Roman);
    }

    // 第一个「要动手」的原因定档。IllegalChar 只记不定档：`04 §7.3.2` 要 `／⧸\` 保留现状，
    // 而 §7.10 第一例（`眠乃よる⧸…`）明确要求这类名字照样进 AI —— 按字面第 3 条返回
    // NORMALIZE_ONLY 会把这一族（参考库里很常见）永久卡在不动档上。
    let first = reasons.iter().copied().find(|r| *r != Reason::IllegalChar);
    let verdict = match first {
        Some(Reason::Locked | Reason::Rejected) => Verdict::Skip,
        Some(r) if NORMALIZE.contains(&r) => Verdict::NormalizeOnly,
        Some(Reason::MtSmell) => Verdict::NeedsRepair,
        Some(Reason::Kana | Reason::Traditional | Reason::Roman) => Verdict::NeedsTranslation,
        _ => Verdict::Compliant,
    };
    Judgment { verdict, reasons }
}

const NORMALIZE: [Reason; 6] = [
    Reason::Whitespace,
    Reason::Wave,
    Reason::HalfFull,
    Reason::OverlyLong,
    Reason::PrefixMixed,
    Reason::DuplicateTitle,
];

impl Judgment {
    /// 要不要送给模型（`04 §7.6` 的入口判据）
    pub fn needs_ai(&self) -> bool {
        matches!(
            self.verdict,
            Verdict::NeedsRepair | Verdict::NeedsTranslation
        )
    }
}

const ILLEGAL: [char; 13] = [
    '\\', '/', ':', '*', '?', '"', '<', '>', '|', '\u{FF0F}', '\u{29F8}', '〘', '〙',
];
/// 需要翻译/修旧的痕迹词（`04 §7.5` ①：`癒し` 逐字直译、语气词硬译、助词未消解）
const SMELL_WORDS: [&str; 4] = ["的癒", "癒的", "哟", "吗"];
const SMELL_TAIL: [char; 4] = ['的', '也', '和', '了'];
const DASH_TAIL: [char; 4] = ['_', '-', '–', '—'];
const WAVES: [char; 4] = ['〜', '~', '∼', '〰'];
/// 半角/全角成对检查：出现半角那侧就算混用（`04 §7.2` 第 6 条）
const HALFFULL_PAIRS: [(char, char); 5] = [('!', '！'), ('?', '？'), (',', '，'), (':', '：'), ('(', '（')];
/// 假名（含长音符与半角片假名）—— 命中即「还有日文没翻」。
/// `・`（U+30FB）落在片假名段里，但它是中文标题也在用的并列符，`04 §7.3.2` 明令保留 → 排除。
pub(crate) fn is_kana(c: char) -> bool {
    c != '・'
        && matches!(
            c,
            '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9F}'
        )
}
/// 汉字（中日通用）。判定树用不到它，但 `04 §7.7` 第 7 条「原文含假名则译文必须含中文」要一个
/// 明确的「中文」口径：日文新字体与简体同形的字（`音`/`時`）也算命中，这条判的是「翻成了汉字」
/// 而不是「翻成了简体中文」，与规范的措辞一致。
pub(crate) fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}
/// 繁体字与日文新字体里跟简体不同形的常用字（`04 §7.2` 第 12 条，也管 §7.5 的「未换中文的日文汉字」）。
/// **不是完整差异表**：没收录的字漏判，漏判的后果只是「这条不出建议」，比拿一张错表误伤专名安全。
const TRADITIONAL: &str = "見覺話語隊録揺縦譲発東車馬鳥點體國學會時間們對於後來聲聞觀閉開關門問長陽陰風雲電書畫統總進運遠資費員單據無變讓認論設許訴調談辭組織結給終經習網線練義聖夢亂靈愛說讀桜專屬歷靜豐龍隱醫";

fn illegal(name: &str) -> bool {
    name.chars().any(|c| ILLEGAL.contains(&c) || (c as u32) < 0x20)
}

fn whitespace(name: &str) -> bool {
    let cs: Vec<char> = name.chars().collect();
    name.contains('\u{3000}')
        || cs.windows(2).any(|w| w[0] == ' ' && w[1] == ' ')
        || name.starts_with(' ')
        || name.ends_with([' ', '.'])
}

fn wave(name: &str) -> bool {
    name.chars().any(|c| WAVES.contains(&c))
}

fn half_full(name: &str) -> bool {
    HALFFULL_PAIRS
        .iter()
        .any(|(half, full)| name.contains(*half) && name.contains(*full))
        || name.contains('!')
        || name.contains('?')
}

fn traditional(name: &str) -> bool {
    // 【繁体中文版】这类版本标签本来就是中文，翻它只会把标签弄坏
    if name.contains("繁体") {
        return false;
    }
    name.chars().any(|c| TRADITIONAL.contains(c))
}

fn roman(name: &str) -> bool {
    if !name.is_ascii() {
        return false;
    }
    // 白名单：这些是设备号/栏目名，不是待译的英文词（`04 §7.2` 第 13 条）
    const ALLOW: [&str; 3] = ["asmr", "cv", "ku100"];
    let lower = name.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty() && !ALLOW.contains(w))
        .count()
        >= 2
}

/// 机翻痕迹（`04 §7.5`）。①词表 + ②里两条**不依赖原文**的启发式；
/// 长度比那条要拿到「原文＋旧译」一对才有意义，等重修链路（`04 §7.7` 后置校验）一起接。
fn mt_smell(name: &str) -> bool {
    if SMELL_WORDS.iter().any(|w| name.contains(w)) {
        return true;
    }
    if name.ends_with(SMELL_TAIL) {
        return true;
    }
    // 「尾部残留分隔符」有两种形状：结尾挂着 `_ -`，或空格贴下划线（`part2 – _3`）。
    // 只有**前缀之外还有正文**时才算残留 —— `Track9_` 那种整段就是编号的，尾巴上的 `_`
    // 是编号自己的分隔符（§7.3.1 逐字节保留），送去模型只会拿到一个空片段。
    if !norm::split_prefix(name).rest.is_empty()
        && (name.ends_with(DASH_TAIL) || name.contains(" _") || name.contains("_ "))
    {
        return true;
    }
    let cs: Vec<char> = name.chars().collect();
    cs.windows(2).any(|w| w[0] == w[1] && matches!(w[0], '的' | '了' | '是'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename::norm::{FILE_MAX, FOLDER_MAX};

    const CTX: Ctx<'_> = Ctx {
        skipped: None,
        folder: None,
        prefix_mixed: false,
    };

    fn v(name: &str) -> Verdict {
        judge(name, FILE_MAX, &CTX).verdict
    }

    #[test]
    fn locked_and_rejected_outrank_everything() {
        let ctx = Ctx {
            skipped: Some(Skipped::Locked),
            ..CTX
        };
        let j = judge("可愛い01_", FOLDER_MAX, &ctx);
        assert_eq!((j.verdict, j.reasons.first()), (Verdict::Skip, Some(&Reason::Locked)));
        // 拒过的优先级一样，只是原因不同（UI 上要能说清是哪回事）
        assert_eq!(
            judge(
                "x",
                FOLDER_MAX,
                &Ctx { skipped: Some(Skipped::Rejected), ..CTX }
            )
            .verdict,
            Verdict::Skip
        );
    }

    #[test]
    fn normalize_only_levels_are_caught_before_the_model() {
        assert!(judge("名称／別名", FILE_MAX, &CTX).reasons.contains(&Reason::IllegalChar));
        assert_eq!(v("名称 01"), Verdict::Compliant);
        assert_eq!(v("名称  空格"), Verdict::NormalizeOnly, "连续空格");
        assert_eq!(v("名称　全角"), Verdict::NormalizeOnly);
        assert_eq!(v("名称."), Verdict::NormalizeOnly, "结尾句点");
        assert_eq!(v("名称〜波浪"), Verdict::NormalizeOnly);
        assert_eq!(v("名称!半角"), Verdict::NormalizeOnly);
        assert_eq!(v("（名称）(半角)"), Verdict::NormalizeOnly, "同一对括号两种宽度都有");
        assert_eq!(v(&"很".repeat(101)), Verdict::NormalizeOnly, "文件主体超限");
    }

    #[test]
    fn illegal_char_alone_is_a_note_not_a_verdict() {
        // `04 §7.3.2`：`／⧸\` 保留现状。既然什么都不改，就不该拦住本该翻译的假名（§7.10 第一例）
        let j = judge("眠乃よる⧸Ear", FILE_MAX, &CTX);
        assert_eq!(j.verdict, Verdict::NeedsTranslation);
        assert_eq!(
            j.reasons,
            [Reason::IllegalChar, Reason::Kana],
            "原因按 §7.2 顺序累积，定档取第一个要动手的"
        );
        assert!(!judge("名称／别名", FILE_MAX, &CTX).needs_ai(), "没有别的痕迹 → 不动");
    }

    #[test]
    fn prefix_mixed_and_duplicate_title_come_from_context() {
        let mixed = Ctx {
            prefix_mixed: true,
            ..CTX
        };
        assert_eq!(judge("01_名前の测试", FILE_MAX, &mixed).verdict, Verdict::NormalizeOnly);
        // 整目录只有一种族时不该报
        assert_eq!(judge("名前の测试", FILE_MAX, &mixed).reasons, vec![Reason::Kana]);
        let dup = Ctx {
            folder: Some("深夜掏耳"),
            ..CTX
        };
        assert!(judge("深夜掏耳_第二夜", FILE_MAX, &dup)
            .reasons
            .contains(&Reason::DuplicateTitle));
    }

    #[test]
    fn mt_smell_examples_from_the_reference_library() {
        // §7.5 ① 那张表里的真实残留
        for name in ["理桜的癒的ー試", "另一侧也掏耳和吗的", "01哟猫鸣馆向。", "part2 – _3", "掏耳_"] {
            assert_eq!(v(name), Verdict::NeedsRepair, "{name}");
        }
        // §7.5 ②「的的/了了」重复
        assert_eq!(v("我的的酱"), Verdict::NeedsRepair);
        assert_eq!(v("05_认知洗牌睡眠法"), Verdict::Compliant);
    }

    #[test]
    fn a_bare_number_is_never_leftover_separator() {
        // §7.3.1 说前缀逐字节保留：`Track9_` 整段就是编号，尾巴的 `_` 是编号自己的分隔符，
        // 判成残留会把一条「没什么可翻」的空片段送进模型
        assert_eq!(v("Track9_"), Verdict::Compliant);
        assert_eq!(v("tr00_"), Verdict::Compliant);
        assert_eq!(v("掏耳_"), Verdict::NeedsRepair, "有正文的才算切割不干净");
    }

    #[test]
    fn translation_levels_split_kana_traditional_and_roman() {
        assert_eq!(v("精霊の魔力"), Verdict::NeedsTranslation);
        let j = judge("可愛音軌", FILE_MAX, &CTX);
        assert_eq!(j.verdict, Verdict::NeedsTranslation);
        assert_eq!(j.reasons, [Reason::Traditional]);
        assert_eq!(v("Ear cleaning"), Verdict::NeedsTranslation);
        assert_eq!(v("ASMR"), Verdict::Compliant, "白名单单词不算罗马词");
        assert_eq!(v("眠乃夜晚"), Verdict::Compliant);
        assert_eq!(v("掏耳・吐息"), Verdict::Compliant, "・ 是并列符不是假名，别当成没翻");
    }

    #[test]
    fn repair_outranks_translation_so_old_bad_translations_get_redone() {
        // 既是机翻残留、又还剩假名：要重修（旧译得当负样本传），不能只当「没翻」
        let j = judge("理桜的癒的ー試", FILE_MAX, &CTX);
        assert_eq!(j.verdict, Verdict::NeedsRepair);
        assert_eq!(j.reasons, [Reason::MtSmell, Reason::Kana, Reason::Traditional]);
    }

    #[test]
    fn a_clean_chinese_name_costs_nothing() {
        for name in ["01_深夜掏耳", "【女仆ASMR】女仆治愈小队～悠闲时间～", "掏耳・吐息_副标题"] {
            assert_eq!(judge(name, FOLDER_MAX, &CTX), Judgment { verdict: Verdict::Compliant, reasons: vec![] }, "{name}");
        }
    }
}
