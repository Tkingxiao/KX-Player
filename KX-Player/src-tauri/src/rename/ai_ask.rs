//! 改名主干的**与模型往来**：拿到一批待翻片段 → 一个片段一次请求 → 过完九道校验的交回
//! [`ai_ledger`](super::ai_ledger) 入库。契约本身（送什么、怎么读回来）在
//! [`ai_name`](super::ai_name)，校验本身在 [`postcheck`](super::postcheck)，库那一侧在
//! [`ai_ledger`](super::ai_ledger)，这里只管中间那段流水 —— 所以本模块不碰 `Connection`。
//!
//! 三条不外溢的口径：
//! 1. **只把过校验的交回去**。`Review` / `Rejected` / 请求失败的一条都不带译文，
//!    [`record`](super::ai_ledger::record) 因此一行也不会写 —— 那张表就是
//!    [`apply`](super::apply) 的通行证，写进去一行等于授权它改一个文件名。
//! 2. 片段之间**互不相干**：`04 §7.6.4` 明令不许把上一条的译文当下一条的上下文（那会让上下文
//!    随条目数线性膨胀，正好踩中本地模型的短板），所以并发是安全的。
//! 3. 端点连不上按 `DEAD_LIMIT` 次判死，剩下的片段直接不再发 —— 与 `ai_chunk` 同一条教训：
//!    接口挂了几百个名字会被逐个敲完，用户既等不到结束也看不到原因。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::ai_strategy::{ModelKind, Strategy};
use crate::model::{AiChatPayload, ChatMessage};
use crate::rename::ai_name::{self, PROMPT_VERSION};
use crate::rename::postcheck::{self, Action, Ctx};

/// 连续几次「端点连不上」就当这批没救了
const DEAD_LIMIT: usize = 3;

/// 一个待翻片段。`refused` 与 `budget` 跟着片段走而不是事后查：同一段文本出现在不同条目上，
/// 用户拒过的候选可能不同（`04 §7.7` 第 9 条），长度预算也不同（前缀长短不一）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub key: String,
    pub src: String,
    pub refused: Vec<String>,
    pub budget: usize,
}

/// 一个片段的最终结论。`dst = None` 就不入库；`blocked` 非空是「过不了校验」，空是「没问到」。
#[derive(Debug, Clone)]
pub struct Outcome {
    pub key: String,
    pub src: String,
    pub dst: Option<String>,
    pub notes: Vec<String>,
    pub blocked: Vec<String>,
}

impl Outcome {
    /// 没拿到结果的样子：请求失败、端点判死、任务没跑完都算 —— `blocked` 留空，
    /// 报告里落进 `failed`（下次点按钮会再来一遍），而不是 `review`（那是「问到了但过不了校验」）。
    fn unanswered(f: &Fragment) -> Self {
        Self { key: f.key.clone(), src: f.src.clone(), dst: None, notes: Vec::new(), blocked: Vec::new() }
    }
}

/// 一次「生成译名」的汇总。`review` 与 `failed` 是**明细**不是计数：烧了 token 却没拿到可用
/// 结果的那些，得让用户看见是哪几条、卡在哪一关。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameReport {
    /// 需要模型的名字条数（按条目计，同一片段出现在两个条目里算两条）
    pub needed: usize,
    /// 其中命中缓存、一个 token 都没花的
    pub cached: usize,
    /// 这次新拿到可用译名的片段数
    pub generated: usize,
    /// 重试过仍过不了校验的（交给人：手工改名，或换个模型再试）
    pub review: Vec<String>,
    /// 请求失败 / 没解析出译文的，下次点按钮会再来一遍
    pub failed: Vec<String>,
    pub model: String,
    pub prompt_version: String,
}

/// 一次请求的外部条件：接入参数一份 + 这一批的判死状态一份。收成一处而不是六个参数，是因为
/// `ask_one` 与 `ask` 都得把这一串原样往下传 —— 散着写的话每加一个参数就要同时改两处签名。
struct Creds<'a> {
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    dead: &'a AtomicUsize,
    aborted: &'a AtomicBool,
}

/// 逐条并发翻译，回**与 `missing` 同序**的结果（报告里的明细要跟用户在屏幕上看到的顺序一致）。
pub async fn translate(
    base_url: &str,
    api_key: &str,
    model: &str,
    kind: ModelKind,
    missing: &[Fragment],
) -> Vec<Outcome> {
    let st = Strategy::resolve(kind, None, None, None);
    let sem = Arc::new(Semaphore::new(st.concurrency.max(1)));
    let dead = Arc::new(AtomicUsize::new(0));
    let aborted = Arc::new(AtomicBool::new(false));
    let mut set = tokio::task::JoinSet::new();
    for (i, f) in missing.iter().enumerate() {
        let (sem, dead, aborted) = (Arc::clone(&sem), Arc::clone(&dead), Arc::clone(&aborted));
        let (base_url, api_key, model) = (base_url.to_string(), api_key.to_string(), model.to_string());
        let f = f.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await;
            let c = Creds { base_url: &base_url, api_key: &api_key, model: &model, dead: &dead, aborted: &aborted };
            (i, ask_one(&c, &f, st).await)
        });
    }
    // 按**下标**回填而不是按键：同一片段带着不同「拒绝集」可以出现两次（键相同），而挪动结果会让
    // 明细整体错位；某个任务被取消没跑完也不能让后面的条目跟着少一行。
    let mut slots: Vec<Option<Outcome>> = (0..missing.len()).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        if let Ok((i, o)) = joined {
            slots[i] = Some(o);
        }
    }
    missing
        .iter()
        .enumerate()
        .map(|(i, f)| slots[i].take().unwrap_or_else(|| Outcome::unanswered(f)))
        .collect()
}

/// 一个片段：一次请求 + 一次校验，有硬伤就带错误说明再要一次（`04 §7.7` 的「退回重试一次」；
/// 第二次仍是硬伤就判 `Review`，不再退 —— 无限重试只会让本地模型一直转）。
async fn ask_one(c: &Creds<'_>, f: &Fragment, st: Strategy) -> Outcome {
    let out = Outcome::unanswered(f);
    let first = ask(c, f, None, st).await;
    let Some(text) = usable(first, &f.src) else { return out };
    let checked = postcheck::check(&text, &ctx_for(f, false));
    if checked.action != Action::Retry {
        return settle(out, checked);
    }
    let hint = postcheck::retry_hint(&checked.blocked);
    let again = ask(c, f, Some(&hint), st).await;
    match usable(again, &f.src) {
        Some(text) => settle(out, postcheck::check(&text, &ctx_for(f, true))),
        None => out,
    }
}

fn settle(mut out: Outcome, checked: postcheck::Checked) -> Outcome {
    if matches!(checked.action, Action::Accepted | Action::Fixed) {
        out.dst = Some(checked.name);
        out.notes = checked.notes;
    } else {
        out.blocked = checked.blocked;
    }
    out
}

fn ctx_for(f: &Fragment, retried: bool) -> Ctx<'_> {
    Ctx { original: &f.src, max_len: f.budget, refused: &f.refused, retried }
}

/// `04 §7.6.3` 末两行：模型把原文还回来 = 没翻（判未翻译，与「解析失败」同一条路，都不入库）；
/// 空 = 失败可重试。这一判在校验之前，因为「原样返回」根本不是一个待校的译文。
fn usable(parsed: Option<String>, src: &str) -> Option<String> {
    parsed.filter(|t| !t.is_empty() && t != src)
}

/// 一次请求：`04 §7.6.1` 的两个字段 —— 逐字节固定的 SYSTEM + 只有片段的 USER。
/// `hint` 只出现在重试那一轮，追在片段后面一行：规范要重试「带错误说明」，而固定的是前缀
/// 不是尾部，KV cache 不受影响。
async fn ask(c: &Creds<'_>, f: &Fragment, hint: Option<&str>, st: Strategy) -> Option<String> {
    let user = match hint {
        Some(h) => format!("{}\n{h}", f.src),
        None => f.src.clone(),
    };
    let payload = AiChatPayload {
        base_url: c.base_url.to_string(),
        api_key: c.api_key.to_string(),
        model: c.model.to_string(),
        messages: vec![
            ChatMessage { role: "system".into(), content: ai_name::SYSTEM_PROMPT.to_string() },
            ChatMessage { role: "user".into(), content: user },
        ],
        temperature: Some(0.3),
        max_tokens: Some(ai_name::max_tokens(&f.src)),
    };
    for i in 0..st.attempts.max(1) {
        if c.aborted.load(Ordering::SeqCst) {
            return None;
        }
        match crate::ai::chat_completion(&payload, st.timeout_ms).await {
            Ok(reply) => {
                c.dead.store(0, Ordering::SeqCst);
                if let Some(text) = ai_name::parse_reply(&reply) {
                    return Some(text);
                }
            }
            Err(e) => {
                if e.is_dead_endpoint() && c.dead.fetch_add(1, Ordering::SeqCst) + 1 >= DEAD_LIMIT {
                    c.aborted.store(true, Ordering::SeqCst);
                }
            }
        }
        if i + 1 < st.attempts.max(1) && st.retry_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(st.retry_delay_ms)).await;
        }
    }
    None
}

/// 报告：把明细摊开给界面。`needed`/`cached` 是问之前算出来的账（在
/// [`plan`](super::ai_ledger::plan) 那一侧），这里只补上问回来之后才知道的部分。
pub fn report(needed: usize, cached: usize, outcomes: &[Outcome], model: &str) -> NameReport {
    let mut rep = NameReport {
        needed,
        cached,
        generated: outcomes.iter().filter(|o| o.dst.is_some()).count(),
        review: Vec::new(),
        failed: Vec::new(),
        model: model.to_string(),
        prompt_version: PROMPT_VERSION.to_string(),
    };
    for o in outcomes.iter().filter(|o| o.dst.is_none()) {
        if o.blocked.is_empty() {
            rep.failed.push(format!("{}：没有问到结果（请求失败或端点连不上）", o.src));
        } else {
            rep.review.push(format!("{}：{}", o.src, o.blocked.join("；")));
        }
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::{report, settle, usable, Fragment, Outcome};
    use crate::rename::ai_name::cache_key;
    use crate::rename::postcheck::{Action, Checked};

    fn frag(src: &str) -> Fragment {
        Fragment { key: cache_key(src), src: src.into(), refused: vec![], budget: 100 }
    }

    fn checked(action: Action, name: &str, blocked: Vec<String>) -> Checked {
        Checked { name: name.into(), action, notes: vec![], blocked }
    }

    /// 「模型把原文还回来」不是一次可入库的结果 —— 它连待校的对象都不是
    #[test]
    fn an_untranslated_reply_is_not_a_result() {
        let f = frag("静夜");
        assert!(usable(Some("静夜".into()), &f.src).is_none(), "原样返回 = 没翻");
        assert!(usable(Some(String::new()), &f.src).is_none());
        assert_eq!(usable(Some("静夜的夜".into()), &f.src).as_deref(), Some("静夜的夜"));
    }

    /// 只有 `Accepted`/`Fixed` 带译文走；`Retry`/`Review`/`Rejected` 只留下卡住的理由
    #[test]
    fn only_accepted_or_fixed_becomes_a_name() {
        let f = frag("静夜");
        let out = Outcome::unanswered(&f);
        assert_eq!(settle(out, checked(Action::Fixed, "静夜", vec![])).dst.as_deref(), Some("静夜"));
        let blocked = settle(Outcome::unanswered(&f), checked(Action::Review, "", vec!["括号数量变了".into()]));
        assert!(blocked.dst.is_none());
        assert_eq!(blocked.blocked, vec!["括号数量变了"]);
    }

    /// `failed`（没问到，下次还会再来一遍）与 `review`（问到了但过不了校验，再来一遍也是白烧
    /// token）必须在报告里分开，否则用户看不出该重试还是该动手
    #[test]
    fn report_separates_a_failed_ask_from_a_blocked_one() {
        let f = frag("静夜");
        let mut no_reply = Outcome::unanswered(&f);
        no_reply.key = "k1".into();
        no_reply.src = "A".into();
        let mut blocked = Outcome::unanswered(&f);
        blocked.key = "k2".into();
        blocked.src = "B".into();
        blocked.blocked = vec!["数字没对上".into()];
        let got = Some("C".into());
        let rep = report(5, 2, &[no_reply, blocked, Outcome {
            key: "k3".into(),
            src: "C".into(),
            dst: got,
            notes: vec![],
            blocked: vec![],
        }], "qwen");
        assert_eq!((rep.needed, rep.cached, rep.generated), (5, 2, 1));
        assert_eq!(rep.failed.len(), 1);
        assert_eq!(rep.review, vec!["B：数字没对上"]);
        assert_eq!(rep.prompt_version, super::PROMPT_VERSION);
    }
}
