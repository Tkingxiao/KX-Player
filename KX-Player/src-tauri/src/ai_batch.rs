//! 块内的**重试与对半拆开**：一个块（≤ chunk_size 行）怎么在失败里救回最多的行。
//!
//! 从 `ai_chunk.rs` 再切一层还是那条 Rust 单文件 ≤400 行的红线，边界也正好是两条失败
//! 对策的分界：
//! - 解析失败（模型没按「编号. 译文」回够 80%）按档位拆：本地模型编号遵循度差，拆开能
//!   救回大部分；外接模型够好，不值得为它多打一倍请求；
//! - **超时一律拆**：批次大小就是超时的变量，对同一批原样重发只会再等一个整超时
//!   （思考型模型实测：15 行 241 s、4 行 10 s）。拆受预算管，预算用完还没救回的行
//!   回退原文并计数 —— 静默回退是「是否正确翻译未知」的根源，计数是它的解药。

use crate::ai_chunk::Hub;
use crate::ai_prompt as prompt;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 一个块内因超时对半拆开的预算。15 行的块正常拆两级就能落地（见实测注），6 是足够宽
/// 的帽 —— 预算用完还没救回来的行回退原文，而不是无限二分烧时间。
const TIMEOUT_SPLIT_BUDGET: usize = 6;

/// 拆不拆这个失败的块（纯函数，方便钉住组合规则）：解析失败只按档位 —— 本地模型
/// 编号遵循度差、拆开能救回大部分；外接模型的编号遵循度够好，不值得为它多打一倍请求。
/// 超时则**两档都拆**：批次大小就是超时的变量，重发同批只会再等一个整超时；
/// 预算用完或已经拆到单行就认了。
fn should_split(bisect: bool, timed_out: bool, len: usize, timeout_budget: usize) -> bool {
    if len <= 1 {
        return false;
    }
    if timed_out {
        timeout_budget > 0
    } else {
        bisect
    }
}

/// 一批文本的翻译：按档位重试；解析失败按档位对半拆开（本地档），超时一律对半拆开。
/// 返回 `(逐行译文, 回退原文的行数)`，`None` = 一批都没翻出来 / 被取消。
pub(crate) async fn translate_range(hub: &Arc<Hub>, texts: &[String]) -> Option<(Vec<String>, usize)> {
    let n = texts.len();
    let mut out: Vec<Option<String>> = vec![None; n];
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    queue.push_back((0, n));
    let mut any_ok = false;
    let mut timeout_splits = TIMEOUT_SPLIT_BUDGET;
    while let Some((a, b)) = queue.pop_front() {
        if hub.cancel.load(Ordering::SeqCst) {
            return None;
        }
        let slice = &texts[a..b];
        let attempt = attempt_batch(hub, slice).await;
        if let Some(list) = attempt.translated {
            for (i, t) in list.into_iter().enumerate() {
                // 空串 = 模型漏了这一行：留在 None 里，收尾统一回退原文并计数
                if !t.is_empty() {
                    out[a + i] = Some(t);
                }
            }
            any_ok = true;
        } else if should_split(hub.st.bisect, attempt.timed_out, b - a, timeout_splits) {
            if attempt.timed_out {
                timeout_splits -= 1;
            }
            let mid = (a + b) / 2;
            queue.push_back((a, mid));
            queue.push_back((mid, b));
        }
    }
    any_ok.then(|| {
        let mut fallback = 0usize;
        let filled: Vec<String> = out
            .into_iter()
            .enumerate()
            .map(|(i, r)| match r {
                Some(t) => t,
                None => {
                    fallback += 1;
                    texts[i].clone()
                }
            })
            .collect();
        (filled, fallback)
    })
}

/// 一次「一批」的结局。`timed_out` 决定调用方走「对半拆开」而不是「重试/放弃」：
/// 对同一批原样重发超时的请求没有意义，模型对这批就是要想这么久。
struct Attempt {
    translated: Option<Vec<String>>,
    timed_out: bool,
}

/// 一批：最多 `attempts` 次尝试，中间退避；每次尝试的失败原因交给 `Hub` 分类计数。
/// 撞到超时就立刻收手返回（不管还剩几次重试）：重试改变的只是等待时长，不是批次大小。
async fn attempt_batch(hub: &Arc<Hub>, texts: &[String]) -> Attempt {
    let payload = crate::model::AiChatPayload {
        base_url: hub.creds.base_url.clone(),
        api_key: hub.creds.api_key.to_string(),
        model: hub.creds.model.clone(),
        messages: vec![
            crate::model::ChatMessage { role: "system".into(), content: prompt::SYSTEM_PROMPT.into() },
            crate::model::ChatMessage { role: "user".into(), content: prompt::numbered_prompt(texts) },
        ],
        temperature: Some(0.3),
        // 字幕一次要回多行，`04 §7.6.4` 那条按单行原文收紧的公式不适用，这里维持不限制
        max_tokens: None,
        model_kind: None,
        api_style: hub.creds.api_style,
    };
    for i in 0..hub.st.attempts.max(1) {
        match crate::ai::chat_completion(&payload, hub.st.timeout_ms).await {
            Ok(reply) => {
                if let Some(parsed) = prompt::parse_numbered_reply(&reply, texts.len()) {
                    hub.succeeded();
                    return Attempt { translated: Some(parsed), timed_out: false };
                }
            }
            Err(e) => {
                if e.is_dead_endpoint() {
                    hub.note_dead(e.text());
                }
                if matches!(e, crate::ai::AiError::Timeout(_)) {
                    hub.note_timeout(e.text());
                    return Attempt { translated: None, timed_out: true };
                }
            }
        }
        if i + 1 < hub.st.attempts && hub.st.retry_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(hub.st.retry_delay_ms)).await;
        }
    }
    Attempt { translated: None, timed_out: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拆分的组合规则：解析失败只按档位（本地拆、外接不拆），超时两档都拆但受预算管，
    /// 单行没得拆。这条规则错了要么烧时间（该拆不拆、同批重试），要么烧 token。
    #[test]
    fn split_rules_distinguish_parse_failure_from_timeout() {
        // 解析失败：本地档拆，外接档不拆（编号遵循度够好，不值得多打一倍请求）
        assert!(should_split(true, false, 15, 0));
        assert!(!should_split(false, false, 15, TIMEOUT_SPLIT_BUDGET));
        // 超时：两档都拆；预算用完就认了；单行没得拆
        assert!(should_split(false, true, 15, TIMEOUT_SPLIT_BUDGET));
        assert!(should_split(true, true, 15, TIMEOUT_SPLIT_BUDGET));
        assert!(!should_split(false, true, 15, 0));
        assert!(!should_split(true, true, 1, TIMEOUT_SPLIT_BUDGET));
    }
}
