//! AI 批量任务的**执行器**：一批文本 → 按档位分块并发 → 每行译文（失败回退原文）。
//!
//! 从 `ai_job.rs` 拆出来是为了守 Rust 单文件 ≤400 行的红线（第 12 批），职责边界也正好切开：
//! `ai_job` 管「任务表 + 事件」，这里管「一批文本怎么变成译文」。三条口径都在本文件：
//! - 参数（批量大小 / 并发 / 超时 / 重试）来自 `ai_strategy::Strategy` 的外接 vs 本地档位；
//! - 逐块 tick 事件限速 —— 一个 300 行字幕就是 20 条 tick，而每条都要 UI 线程执行一次脚本，
//!   前端还不消费它们（只认 file-* 与 cancelled），发得越快越是纯开销；
//! - 连续「端点连不上」够 `DEAD_LIMIT` 次就自动按停止处理：否则接口一挂，几百个文件会被
//!   逐个敲一遍连接拒绝，用户既等不到结束也看不到原因。

use crate::ai_job::{progress, Loc};
use crate::ai_prompt as prompt;
use crate::ai_strategy::{ModelKind, Strategy};
use crate::subtitles as subs;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 连续多少次「端点连不上」就判定这批没救了。3 次 = 已经重试过，仍然全是连接失败。
const DEAD_LIMIT: usize = 3;
/// 逐块 tick 的最小间隔
const TICK_GAP: Duration = Duration::from_millis(500);

/// 模型接入三件套（字段名与 `AiChatPayload` 对齐）
pub struct Creds {
    base_url: String,
    api_key: String,
    model: String,
}

pub fn creds_of(base_url: &str, api_key: &str, model: &str) -> Creds {
    Creds { base_url: base_url.to_string(), api_key: api_key.to_string(), model: model.to_string() }
}

/// 一次任务运行期跨线程共享的部分：会被 `Arc::clone` 进每个请求任务。
pub struct Hub {
    creds: Creds,
    pub st: Strategy,
    pub cancel: Arc<AtomicBool>,
    /// 连续「端点连不上」的次数；任何一次成功都清零
    dead: AtomicUsize,
    /// 够数之后就锁死，**不再被清零**：判定失联后仍可能有在途请求成功回来，
    /// 那不能把「这批其实中途停了」这件事洗掉（否则用户只看到「翻译完成」）。
    aborted: AtomicBool,
    dead_reason: Mutex<Option<String>>,
    /// 投递失败只记一条：真灌满了的时候，记日志本身也会拖慢任务线程
    emit_err_logged: AtomicBool,
}

impl Hub {
    pub fn for_job(
        creds: Creds,
        kind: ModelKind,
        chunk_size: Option<usize>,
        concurrency: Option<usize>,
        merge_lines: Option<usize>,
        cancel: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            creds,
            st: Strategy::resolve(kind, chunk_size, concurrency, merge_lines),
            cancel,
            dead: AtomicUsize::new(0),
            aborted: AtomicBool::new(false),
            dead_reason: Mutex::new(None),
            emit_err_logged: AtomicBool::new(false),
        })
    }

    fn emit(&self, app: &AppHandle, p: crate::ai_job::AiProgress) {
        if let Err(e) = app.emit("ai:progress", p) {
            if !self.emit_err_logged.swap(true, Ordering::Relaxed) {
                // 投递不出去 = UI 线程的消息队列被灌满了。以前这里 `let _ =` 吞掉，
                // 用户看到的是「进度不动了」，日志里一个字都没有。
                crate::paths::append_log(&format!("[ai] ai:progress 投递失败: {e}"));
            }
        }
    }

    /// 记一次「连不上」。够数了就等于用户按了停止 —— 后面的文件没必要再敲。
    fn note_dead(&self, why: String) {
        if self.dead.fetch_add(1, Ordering::SeqCst) + 1 >= DEAD_LIMIT {
            *self.dead_reason.lock() = Some(why);
            self.aborted.store(true, Ordering::SeqCst);
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn succeeded(&self) {
        self.dead.store(0, Ordering::SeqCst);
    }

    /// 失联到值得中断整条队列时，回原因（`ai_job` 把它放进收尾事件的 error）
    pub fn fatal(&self) -> Option<String> {
        self.aborted.load(Ordering::SeqCst).then(|| self.dead_reason.lock().clone().unwrap_or_default())
    }
}

/// 一次任务运行期的上下文（逐块循环与单个文件都用同一份）
pub struct Run<'a> {
    app: &'a AppHandle,
    task_id: u64,
    pub hub: Arc<Hub>,
}

impl<'a> Run<'a> {
    pub fn new(app: &'a AppHandle, task_id: u64, hub: Arc<Hub>) -> Self {
        Self { app, task_id, hub }
    }

    pub fn emit(&self, p: crate::ai_job::AiProgress) {
        self.hub.emit(self.app, p)
    }
}

/// 翻译单个字幕文件。Ok(None) = 被取消。
pub async fn translate_one(run: &Run<'_>, loc: Loc<'_>, path: &str) -> Result<Option<String>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let doc = subs::parse(&content, &ext).ok_or_else(|| format!("不支持的字幕格式: .{ext}"))?;
    if doc.segment_count() == 0 {
        return Err("没有可翻译的文本".into());
    }
    let translated = match translate_chunks(run, loc, doc.segments()).await {
        Some(list) => list,
        None => return Ok(None),
    };
    let out_path = prompt::translated_path(path, doc.kind().as_str());
    std::fs::write(&out_path, doc.render(&translated)).map_err(|e| format!("写入失败: {e}"))?;
    Ok(Some(out_path))
}

/// 逐块并发翻译，块级取消。**返回一定与 `texts` 等长**：失败的块回退原文，绝不吐空行。
/// `None` = 被取消（调用方别写盘）。
pub async fn translate_chunks(run: &Run<'_>, loc: Loc<'_>, texts: &[String]) -> Option<Vec<String>> {
    let st = run.hub.st;
    let ranges = prompt::chunk_ranges(texts.len(), st.chunk_size);
    let total = ranges.len();
    let mut results: Vec<Option<Vec<String>>> = vec![None; total];

    let sem = Arc::new(tokio::sync::Semaphore::new(st.concurrency));
    let mut set = tokio::task::JoinSet::new();
    for (idx, (start, end)) in ranges.iter().copied().enumerate() {
        let permit = Arc::clone(&sem);
        let hub = Arc::clone(&run.hub);
        let texts = texts[start..end].to_vec();
        set.spawn(async move {
            let _guard = permit.acquire().await;
            if hub.cancel.load(Ordering::SeqCst) {
                return (idx, None);
            }
            (idx, translate_range(&hub, &texts).await)
        });
    }
    // tick 只当心跳：终结态事件（file-* / cancelled / queue-finished）由调用方发，必到
    let mut last_tick = Instant::now();
    let mut done = 0usize;
    while let Some(joined) = set.join_next().await {
        let Ok((idx, out)) = joined else { continue };
        done += 1;
        if let Some(translated) = out {
            results[idx] = Some(translated);
        }
        if done == total || last_tick.elapsed() >= TICK_GAP {
            last_tick = Instant::now();
            let mut p = progress(run.task_id, "chunk-done", loc);
            p.chunk_done = done;
            p.chunk_total = total;
            p.segment_count = texts.len();
            run.emit(p);
        }
        if run.hub.cancel.load(Ordering::SeqCst) {
            set.abort_all();
            return None;
        }
    }
    if run.hub.cancel.load(Ordering::SeqCst) {
        return None;
    }
    let flat: Vec<String> = results
        .into_iter()
        .enumerate()
        .flat_map(|(i, r)| {
            let (start, end) = ranges[i];
            r.unwrap_or_else(|| texts[start..end].to_vec()).into_iter()
        })
        .collect();
    Some(prompt::fill_missing(flat, texts))
}

/// 一批文本的翻译：按档位重试；本地档位解析失败时**对半拆开再试** —— 小模型给 8 行更容易漏
/// 编号，拆开还能救回大部分，而不是整批退回原文。返回 None = 一批都没翻出来 / 被取消。
async fn translate_range(hub: &Arc<Hub>, texts: &[String]) -> Option<Vec<String>> {
    let n = texts.len();
    let mut out: Vec<Option<String>> = vec![None; n];
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    queue.push_back((0, n));
    let mut any_ok = false;
    while let Some((a, b)) = queue.pop_front() {
        if hub.cancel.load(Ordering::SeqCst) {
            return None;
        }
        let slice = &texts[a..b];
        if let Some(list) = attempt_batch(hub, slice).await {
            for (i, t) in list.into_iter().enumerate() {
                out[a + i] = Some(t);
            }
            any_ok = true;
        } else if hub.st.bisect && b - a > 1 {
            let mid = (a + b) / 2;
            queue.push_back((a, mid));
            queue.push_back((mid, b));
        }
    }
    any_ok.then(|| {
        out.into_iter()
            .enumerate()
            .map(|(i, r)| r.unwrap_or_else(|| texts[i].clone()))
            .collect()
    })
}

/// 一批：最多 `attempts` 次尝试，中间退避；每次尝试的失败原因交给 `Hub` 分类计数。
async fn attempt_batch(hub: &Arc<Hub>, texts: &[String]) -> Option<Vec<String>> {
    let payload = crate::model::AiChatPayload {
        base_url: hub.creds.base_url.clone(),
        api_key: hub.creds.api_key.clone(),
        model: hub.creds.model.clone(),
        messages: vec![
            crate::model::ChatMessage { role: "system".into(), content: prompt::SYSTEM_PROMPT.into() },
            crate::model::ChatMessage { role: "user".into(), content: prompt::numbered_prompt(texts) },
        ],
        temperature: Some(0.3),
    };
    for i in 0..hub.st.attempts.max(1) {
        match crate::ai::chat_completion(&payload, hub.st.timeout_ms).await {
            Ok(reply) => {
                if let Some(mut parsed) = prompt::parse_numbered_reply(&reply, texts.len()) {
                    for (j, t) in parsed.iter_mut().enumerate() {
                        if t.is_empty() {
                            *t = texts[j].clone();
                        }
                    }
                    hub.succeeded();
                    return Some(parsed);
                }
            }
            Err(e) => {
                if e.is_dead_endpoint() {
                    hub.note_dead(e.text());
                }
            }
        }
        if i + 1 < hub.st.attempts && hub.st.retry_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(hub.st.retry_delay_ms)).await;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_job::AiProgress;

    fn hub() -> Arc<Hub> {
        Hub::for_job(
            creds_of("http://x", "", "m"),
            ModelKind::Remote,
            None,
            None,
            None,
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// 端点失联要的是「早停」，不是「每个文件都失败一遍」：够数之前不动，够数之后自动停止
    #[test]
    fn dead_endpoint_stops_the_queue_after_the_limit() {
        let h = hub();
        assert_eq!(h.fatal(), None, "还没敲过就不算失联");
        for _ in 0..DEAD_LIMIT - 1 {
            h.note_dead("请求失败: 连接被拒绝".into());
        }
        assert!(!h.cancel.load(Ordering::SeqCst), "差一次就停，不能提前");
        assert_eq!(h.fatal(), None);
        h.note_dead("请求失败: 连接被拒绝".into());
        assert!(h.cancel.load(Ordering::SeqCst), "够数就按停止处理");
        assert_eq!(h.fatal().as_deref(), Some("请求失败: 连接被拒绝"), "原因要能带给用户");
    }

    #[test]
    fn a_single_success_clears_the_dead_streak() {
        let h = hub();
        h.note_dead("请求失败: 连接被拒绝".into());
        h.succeeded();
        for _ in 0..DEAD_LIMIT - 1 {
            h.note_dead("请求失败: 连接被拒绝".into());
        }
        assert!(!h.cancel.load(Ordering::SeqCst), "中间成功过就不算连续失联");
    }

    /// 判定失联之后，在途请求仍可能成功回来 —— 那不能把中断原因洗掉
    #[test]
    fn the_abort_reason_survives_a_late_success() {
        let h = hub();
        for _ in 0..DEAD_LIMIT {
            h.note_dead("请求失败: 连接被拒绝".into());
        }
        h.succeeded();
        assert_eq!(h.fatal().as_deref(), Some("请求失败: 连接被拒绝"), "已经按中断处理了，原因得留住");
    }

    /// 编译级防线：`emit` 的载荷必须仍是 ai_job 的那个事件结构（字段变了这里就红）
    #[test]
    fn progress_event_still_carries_the_task_scope() {
        let p: AiProgress = progress(7, "file-start", Loc::new("a.lrc", 1, 3));
        assert_eq!((p.task_id, p.state.as_str(), p.file_index, p.file_count), (7, "file-start", 1, 3));
    }
}
