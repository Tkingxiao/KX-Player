//! 改名主干的**请求契约**（`04 §7.6.1`~`§7.6.4`）：送给模型的那段文本长什么样、模型回的
//! 那行文本怎么读出来。跟模型往来的那一半在 [`ai_ask`](super::ai_ask)。
//!
//! 拆成两半的理由是**能测的与不能测的要分开**：本文件全是纯函数，片段口径、长度预算、`max_tokens`
//! 公式、容错解析四条都能写成断言；网络与库都不在这里。契约的三处口径都在本文件：
//! - SYSTEM 逐字节固定 —— 前缀稳定才吃得到本地模型的 KV cache（`04 §7.6.2` 的速度账）；
//! - USER 只有待翻片段，没有「请翻译以下内容」、没有编号、没有类型标注；
//! - `max_tokens` 按原文长度收紧（`04 §7.6.4`），防本地模型进入复读。
//!
//! 片段不含序号前缀与扩展名 —— 那两样由 [`fragment`] 剥掉、落地前由 [`rebuild`] 原样拼回。

use crate::rename::norm::{self, FILE_MAX, FOLDER_MAX};
use crate::rename::preview::{Item, Kind};

/// 提示词版本。`04 §7.6.1` 的 `PROMPT_VERSION = 3`，前缀 `name-` 区分工种 —— 字幕那侧是
/// `subtitle-1`，两版措辞互不牵连对方的缓存。**改了 SYSTEM 一个字就必须涨号**（旧译整体作废）。
pub const PROMPT_VERSION: &str = "name-3";

/// `04 §7.6.1` 的六条规则，逐字节照抄。**里面不许有变量**：一旦把「作品上下文」这类变量塞进
/// system，每个请求的前缀就不再相同，等于每次从头 prefill 一遍。
pub const SYSTEM_PROMPT: &str = r#"你是日文/英文到简体中文的名称翻译器。输入是一段需要翻译的文本，输出它的简体中文译文。

规则：
1. 只输出译文本身，不要解释、不要引号、不要 Markdown、不要换行。
2. 不要增加原文没有的信息，不要写括号注释。
3. 原文中的数字原样保留：01 不要写成 1。
4. 人名、作品名、设备名（如 KU100、ASMR、CV）若我未给出中文，保留原文。
5. 读起来要像中文原生表达，不要逐字直译，助词要消解掉。
6. 若原文本身已是中文或无法翻译，原样输出。"#;

/// 送给模型的那段正文：本地规范化后的名字，去掉序号前缀、再去掉扩展名。
pub fn fragment(item: &Item) -> String {
    let (stem, _) = split_ext(&item.target);
    norm::split_prefix(stem).rest.trim().to_string()
}

/// 正文的长度预算：整名上限扣掉前缀。扩展名不算 —— `FILE_MAX` 管的是主体（`04 §7.2` 第 7 条）。
pub fn body_budget(item: &Item) -> usize {
    let max = if item.kind == Kind::Folder { FOLDER_MAX } else { FILE_MAX };
    let (stem, _) = split_ext(&item.target);
    max.saturating_sub(norm::split_prefix(stem).prefix.chars().count())
}

/// 把过完校验的正文拼回一个整名：原前缀 + 正文 + 原扩展名。
/// 这里按**这个条目自己**的预算再收一次长度：缓存里那条可能是别的条目（前缀更短、预算更宽）
/// 留下的，不重收就会写出一个超过 `04 §7.2` 第 7 条的名字。
pub fn rebuild(item: &Item, body: &str) -> String {
    let (stem, ext) = split_ext(&item.target);
    let prefix = norm::split_prefix(stem).prefix;
    let budget = body_budget(item);
    let kept = if body.chars().count() > budget { norm::truncate(body, budget) } else { body.to_string() };
    format!("{prefix}{kept}{ext}")
}

/// `04 §7.6.4`：`max_tokens = ceil(原文字符数 × 1.4) + 8`。整数写法避免浮点。
pub fn max_tokens(src: &str) -> u32 {
    let n = src.chars().count() as u32;
    (n * 14).div_ceil(10) + 8
}

/// 内容寻址的缓存键（`04 §7.6.2`）：换模型不作废，涨 `PROMPT_VERSION` 才整体作废。
pub fn cache_key(src: &str) -> String {
    crate::ai_cache::hash(PROMPT_VERSION, src.as_bytes())
}

/// `04 §7.6.3` 的容错解析：剥围栏、剥引号、剥「译文：」前缀、多行只取第一条有内容的。
/// 一律按「格式不完美不算失败」处理 —— 小模型就是这样，判它失败只会让用户白白烧 token。
pub fn parse_reply(reply: &str) -> Option<String> {
    let mut s = reply.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let body = rest.split_once("```").map_or(rest, |(a, _)| a);
        // 围栏后那行的语言标记（```js）不是译文
        s = body.split_once('\n').map_or(body, |(_, b)| b).trim();
    }
    let first = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let first = first.strip_prefix("译文：").or_else(|| first.strip_prefix("翻译：")).unwrap_or(first).trim();
    let first = first.trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '「' | '」'));
    let first = first.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// 去掉扩展名。判据必须与 `preview::split_ext` 一致，否则「送给模型的片段」与「预览里显示的
/// 名字」会对不上 —— 那份是私有的，两处各自持有、由 `preview` 的用例钉住同一个口径。
fn split_ext(name: &str) -> (&str, &str) {
    match std::path::Path::new(name).extension() {
        Some(x) => {
            let x = x.to_string_lossy();
            let cut = name.len() - x.len() - 1;
            (&name[..cut], &name[cut..])
        }
        None => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::{body_budget, cache_key, fragment, max_tokens, parse_reply, rebuild, SYSTEM_PROMPT};
    use crate::rename::judge::Verdict;
    use crate::rename::preview::{Item, Kind};

    fn item(name: &str, target: &str, kind: Kind) -> Item {
        Item {
            path: format!("T:/x/{name}"),
            name: name.into(),
            target: target.into(),
            kind,
            verdict: Verdict::NeedsTranslation,
            reasons: vec![],
            changed: true,
            needs_ai: true,
            suggested: None,
            conflict: None,
        }
    }

    /// 前缀与扩展名都不许进模型，也不许在拼回时丢掉
    #[test]
    fn fragment_strips_prefix_and_extension_and_rebuild_puts_them_back() {
        let f = item("01_癒しの音.mp3", "01_治愈之音.mp3", Kind::File);
        assert_eq!(fragment(&f), "治愈之音");
        assert_eq!(rebuild(&f, "温柔嗓音"), "01_温柔嗓音.mp3");
        let d = item("特典2_思い出", "特典2_回忆", Kind::Folder);
        assert_eq!(fragment(&d), "回忆");
        assert_eq!(rebuild(&d, "青春"), "特典2_青春");
    }

    /// 长度预算扣前缀：文件夹 120 / 文件 100，扩展名不占预算
    #[test]
    fn body_budget_deducts_the_prefix() {
        let d = item("特典2_思い出", "特典2_思い出", Kind::Folder);
        assert_eq!(body_budget(&d), 120 - "特典2_".chars().count());
        let f = item("01_a.mp3", "01_a.mp3", Kind::File);
        assert_eq!(body_budget(&f), 100 - 3);
    }

    /// 缓存里那条可能是宽预算的条目留下的，拼回时按这个条目自己的预算收：前缀 + 正文合起来
    /// 顶到 `04 §7.2` 第 7 条那个数（100 / 120 管的是**正文整体**，扩展名另算、不占它）
    #[test]
    fn rebuild_respects_this_item_s_own_budget() {
        let f = item("01_x.mp3", "01_x.mp3", Kind::File);
        let name = rebuild(&f, &"字".repeat(120));
        assert_eq!(name.chars().count(), 100 + 4, "正文收进 100 以内，扩展名照旧：{name}");
        assert!(name.ends_with(".mp3"), "截断不许把扩展名截掉：{name}");
    }

    /// `04 §7.6.4` 的公式：向上取整再加 8
    #[test]
    fn max_tokens_follows_the_formula() {
        assert_eq!(max_tokens("abc"), 13);
        assert_eq!(max_tokens(""), 8);
        assert_eq!(max_tokens("癒しの音"), 14);
    }

    /// `04 §7.6.3`：围栏、引号、「译文：」、多余空行都不算失败
    #[test]
    fn parse_reply_tolerates_small_model_noise() {
        assert_eq!(parse_reply("  静夜  ").as_deref(), Some("静夜"));
        assert_eq!(parse_reply("\"静夜\"").as_deref(), Some("静夜"));
        assert_eq!(parse_reply("译文：静夜").as_deref(), Some("静夜"));
        assert_eq!(parse_reply("```js\n静夜\n```").as_deref(), Some("静夜"));
        assert_eq!(parse_reply("\n\n静夜\n再解释一句").as_deref(), Some("静夜"));
        assert_eq!(parse_reply("   "), None);
    }

    /// 缓存键按内容寻址：同片段同键，提示词版本进键（换版本整体作废）
    #[test]
    fn cache_key_is_content_addressed() {
        assert_eq!(cache_key("癒し"), cache_key("癒し"));
        assert_ne!(cache_key("癒し"), cache_key("癒し "), "多一个空格就是另一个片段");
        assert_ne!(crate::ai_cache::hash("name-2", "癒し".as_bytes()), cache_key("癒し"));
    }

    /// SYSTEM 里不许出现变量位（前缀逐字节稳定才吃得到 KV cache）
    #[test]
    fn system_prompt_has_no_slots() {
        assert!(!SYSTEM_PROMPT.contains("{{"), "SYSTEM 里出现了模板占位符");
        assert!(SYSTEM_PROMPT.lines().count() >= 7, "六条规则 + 首句，少了就是照抄漏了行");
    }
}
