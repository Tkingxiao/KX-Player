//! AI 批量任务的**事件载荷**：`ai:progress` 一条流上两个任务（字幕文件 / 纯文本）共用的
//! 位置与进度结构。
//!
//! 从 `ai_job.rs` 拆出来是守 Rust 单文件 ≤400 行的红线（第 12 批）：超时对半拆开、
//! 回退计数、force 重译三件事进任务层时，事件结构正好切开成独立一层 —— 它是前后端
//! 契约的一部分（`src/contracts/dto.ts` 有对应镜像），字段口径的变更两端都要动。

/// 事件里定位「哪一个单元的哪一块」，两个任务的字段口径一致
#[derive(Debug, Clone, Copy)]
pub struct Loc<'a> {
    path: &'a str,
    file_index: usize,
    file_count: usize,
}

impl<'a> Loc<'a> {
    pub fn new(path: &'a str, file_index: usize, file_count: usize) -> Self {
        Self { path, file_index, file_count }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProgress {
    pub task_id: u64,
    /// file-start | chunk-done | file-done | file-failed | cancelled | queue-finished
    pub state: String,
    pub path: String,
    pub file_index: usize,
    pub file_count: usize,
    pub chunk_done: usize,
    pub chunk_total: usize,
    pub segment_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_path: Option<String>,
    /// `file-failed` = 这个文件为什么没翻成；`queue-finished` = 整条队列**为什么提前结束**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 仅 `run_texts` 的收尾事件带：与请求等长、缺项已回退原文
    pub translated: Option<Vec<String>>,
    /// 这批里有多少行最终回退了原文（>0 才出现）。「翻译跑完」和「翻译翻好了」的差值
    /// 必须让用户看见 —— 静默回退就是「是否正确翻译未知」的根源。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_lines: Option<usize>,
}

pub(crate) fn progress(task_id: u64, state: &str, loc: Loc<'_>) -> AiProgress {
    AiProgress {
        task_id,
        state: state.to_string(),
        path: loc.path.to_string(),
        file_index: loc.file_index,
        file_count: loc.file_count,
        chunk_done: 0,
        chunk_total: 0,
        segment_count: 0,
        out_path: None,
        error: None,
        translated: None,
        fallback_lines: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「部分未翻」是收尾可见性的核心信号：>0 必须上线，=0 必须整键消失，
    /// 让「翻干净了」的文件在事件形状上与过去完全一致。
    #[test]
    fn fallback_lines_appear_only_when_there_is_something_to_report() {
        let mut p = progress(1, "file-done", Loc::new("a.lrc", 0, 1));
        assert!(serde_json::to_value(&p).unwrap().get("fallbackLines").is_none());
        p.fallback_lines = Some(3);
        assert_eq!(serde_json::to_value(&p).unwrap()["fallbackLines"], 3);
    }
}
