//! AI 批量任务的**翻译策略**：外接模型与本地模型走两套参数（第 12 批）。
//!
//! 为什么不是一个数字的事：本地模型（Ollama / llama.cpp 这类）的瓶颈和云端 API 完全不同 ——
//! 一次只扛得住一路请求（并发 2 会把 KV cache 和内存一起吃掉，两边都变慢），首 token 可能要
//! 几分钟（120 s 超时会把刚加载完权重的请求判成失败），而小模型给它的行数一多就开始漏编号、
//! 把「1. 2. 3.」写成一段散文。所以三处分开：批量大小、并发/超时、以及解析失败后怎么回收。

use serde::Deserialize;

/// 用户选的模型接入类型。缺省 `Remote` = 第 11 批的行为，不选就等于没变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    /// 外接（OpenAI 兼容云端接口）
    #[default]
    Remote,
    /// 本地部署（Ollama 等）
    Local,
}

/// 一次任务运行期的策略参数。小、Copy，随 `Run` 传下去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strategy {
    /// 一次请求合并多少行（本地模型 = 用户选的「一次合并几条」）
    pub chunk_size: usize,
    /// 同时在飞几个请求
    pub concurrency: usize,
    /// 一块最多试几次
    pub attempts: usize,
    /// 相邻两次尝试之间的退避，避免端点挂了还以毫秒级频率敲它
    pub retry_delay_ms: u64,
    pub timeout_ms: u64,
    /// 一批解析不出编号时对半拆开再试，而不是一句「翻不出来」退回原文
    pub bisect: bool,
}

/// 本地模型一次合并行数的上下限（前端的选择框按这个口径做）
pub const LOCAL_MERGE_MIN: usize = 2;
pub const LOCAL_MERGE_MAX: usize = 10;
const LOCAL_MERGE_DEFAULT: usize = 4;

impl Strategy {
    /// `chunk_size` / `concurrency` 是契约里的可选覆盖项：**显式传入优先于档位缺省**，
    /// 这样「按档位换策略」和「调用方自己精确控制」两种用法都留着。
    pub fn resolve(
        kind: ModelKind,
        chunk_size: Option<usize>,
        concurrency: Option<usize>,
        merge_lines: Option<usize>,
    ) -> Self {
        let base = match kind {
            // 外接：沿用第 11 批的实测参数，不动语义
            ModelKind::Remote => Self {
                chunk_size: 15,
                concurrency: 2,
                attempts: 3,
                retry_delay_ms: 250,
                timeout_ms: 120_000,
                bisect: false,
            },
            // 本地：一次一路请求、给足加载时间、批量小且带二分回收
            ModelKind::Local => Self {
                chunk_size: merge_lines.unwrap_or(LOCAL_MERGE_DEFAULT).clamp(LOCAL_MERGE_MIN, LOCAL_MERGE_MAX),
                concurrency: 1,
                attempts: 2,
                retry_delay_ms: 400,
                timeout_ms: 300_000,
                bisect: true,
            },
        };
        Self {
            chunk_size: chunk_size.unwrap_or(base.chunk_size).clamp(1, 200),
            concurrency: concurrency.unwrap_or(base.concurrency).clamp(1, 8),
            ..base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> ModelKind {
        serde_json::from_str::<ModelKind>(s).unwrap()
    }

    #[test]
    fn remote_profile_keeps_batch_11_numbers() {
        let st = Strategy::resolve(ModelKind::Remote, None, None, None);
        assert_eq!((st.chunk_size, st.concurrency, st.attempts), (15, 2, 3));
        assert_eq!(st.timeout_ms, 120_000);
        assert!(!st.bisect, "外接模型的编号遵循度够好，不值得为它多打一倍请求");
    }

    #[test]
    fn local_profile_serializes_and_gives_time_to_load() {
        let st = Strategy::resolve(ModelKind::Local, None, None, None);
        assert_eq!((st.chunk_size, st.concurrency, st.attempts), (4, 1, 2));
        assert_eq!(st.timeout_ms, 300_000, "本地首次加载权重要几分钟");
        assert!(st.bisect);
        assert!(st.retry_delay_ms > 0, "端点挂了也不能毫秒级反复敲");
    }

    #[test]
    fn merge_lines_clamps_to_the_offered_range() {
        assert_eq!(Strategy::resolve(ModelKind::Local, None, None, Some(1)).chunk_size, LOCAL_MERGE_MIN);
        assert_eq!(Strategy::resolve(ModelKind::Local, None, None, Some(99)).chunk_size, LOCAL_MERGE_MAX);
        assert_eq!(Strategy::resolve(ModelKind::Local, None, None, Some(8)).chunk_size, 8);
        // 选的是本地档位，显式覆盖仍然说话：并发被人指定就听人的
        let st = Strategy::resolve(ModelKind::Local, Some(30), Some(3), Some(5));
        assert_eq!((st.chunk_size, st.concurrency), (30, 3));
    }

    #[test]
    fn model_kind_wire_format_is_lowercase_and_optional() {
        assert_eq!(spec("\"local\""), ModelKind::Local);
        assert_eq!(spec("\"remote\""), ModelKind::Remote);
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct S {
            #[serde(default)]
            model_kind: ModelKind,
        }
        assert_eq!(serde_json::from_str::<S>("{}").unwrap().model_kind, ModelKind::Remote);
        assert_eq!(serde_json::from_str::<S>(r#"{"modelKind":"local"}"#).unwrap().model_kind, ModelKind::Local);
    }
}
