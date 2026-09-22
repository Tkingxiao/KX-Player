//! AI 批量任务：字幕翻译从「前端逐块 await」改成「Rust 侧任务 + ai:progress 事件」（B3）。
//!
//! 与转换队列（fftools::run_convert_queue）同一套路：任务号 + 取消标志表 + 事件广播。
//! 为什么要搬：旧实现每翻译 15 行就要过一次 IPC（readTextFile → 逐块 aiChat → 用 base64 写回），
//! 一个 300 行的字幕文件是 20+ 次往返，中途关窗口就丢进度、也取消不了；现在整条链路在 Rust
//! 里跑，前端只负责「启动 + 收事件 + 画进度」。
//!
//! 本文件只管「任务表 + 事件流 + 队列循环」，「一批文本怎么翻」在 `ai_chunk.rs`，
//! 「外接 / 本地各自的参数档」在 `ai_strategy.rs`。两个任务共用同一条 `ai:progress`：
//! - `run_translate`：一批字幕文件 → 各自的 `.zh.*` 译文文件（写盘）
//! - `run_texts`：一批纯文本（AI 页的「文件夹名翻译」）→ 只在收尾事件里回传译文，不落盘

use crate::ai_chunk::{creds_of, translate_chunks, translate_one, Hub, Run};
use crate::ai_strategy::ModelKind;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap as StdHashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTranslateSpec {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 待翻译的字幕文件路径（扫描由 subtitle_scan_dir 负责，任务只认路径）
    pub paths: Vec<String>,
    /// 模型档位，缺省 `remote` = 第 11 批的参数
    #[serde(default)]
    pub model_kind: ModelKind,
    /// 本地模型一次合并几条（2–10），只有 `modelKind = local` 时起作用
    pub merge_lines: Option<usize>,
    /// 显式覆盖档位（前端两页都不传，留着给调用方精确控制）
    pub chunk_size: Option<usize>,
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTextsSpec {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 待翻译的纯文本，顺序即回传顺序
    pub texts: Vec<String>,
    #[serde(default)]
    pub model_kind: ModelKind,
    pub merge_lines: Option<usize>,
    pub chunk_size: Option<usize>,
    pub concurrency: Option<usize>,
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
}

static AI_SEQ: AtomicU64 = AtomicU64::new(0);
static AI_FLAGS: Lazy<Mutex<StdHashMap<u64, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(StdHashMap::new()));

/// 一个任务的两种载荷，用同一个入口起线程
enum Job {
    Files(AiTranslateSpec),
    Texts(AiTextsSpec),
}

/// 启动字幕翻译任务，返回 taskId（进度与结果都走 ai:progress 事件）
pub fn run_translate(app: AppHandle, spec: AiTranslateSpec) -> u64 {
    let units = spec.paths.len();
    spawn_job(app, units, Job::Files(spec))
}

/// 启动纯文本批量翻译任务（AI 页的「文件夹名翻译」），返回 taskId
pub fn run_texts(app: AppHandle, spec: AiTextsSpec) -> u64 {
    let units = spec.texts.len();
    spawn_job(app, units, Job::Texts(spec))
}

fn spawn_job(app: AppHandle, units: usize, job: Job) -> u64 {
    let task_id = AI_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    AI_FLAGS.lock().insert(task_id, Arc::clone(&cancel));
    std::thread::spawn(move || {
        match crate::fftools::runtime() {
            Ok(rt) => match &job {
                Job::Files(spec) => rt.block_on(run_all(&app, spec, task_id, &cancel)),
                Job::Texts(spec) => rt.block_on(run_all_texts(&app, spec, task_id, &cancel)),
            },
            Err(e) => {
                let mut p = progress(task_id, "queue-finished", Loc::new("", 0, units));
                p.error = Some(format!("任务运行时不可用：{e}"));
                if let Err(emit_err) = app.emit("ai:progress", p) {
                    crate::paths::append_log(&format!("[ai] 任务起不来且事件也投不出去: {emit_err}"));
                }
            }
        }
        AI_FLAGS.lock().remove(&task_id);
    });
    task_id
}

/// 取消任务（幂等）：正在飞的那一块会跑完，后续块与后续文件不再开始 —— 块级取消粒度
pub fn cancel(task_id: u64) -> bool {
    match AI_FLAGS.lock().get(&task_id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
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
    }
}

async fn run_all(app: &AppHandle, spec: &AiTranslateSpec, task_id: u64, cancel: &Arc<AtomicBool>) {
    let hub = Hub::for_job(
        creds_of(&spec.base_url, &spec.api_key, &spec.model),
        spec.model_kind,
        spec.chunk_size,
        spec.concurrency,
        spec.merge_lines,
        Arc::clone(cancel),
    );
    let run = Run::new(app, task_id, hub);
    let file_count = spec.paths.len();
    for (i, path) in spec.paths.iter().enumerate() {
        let loc = Loc::new(path, i, file_count);
        if run.hub.cancel.load(Ordering::SeqCst) {
            run.emit(progress(task_id, "cancelled", loc));
            continue;
        }
        run.emit(progress(task_id, "file-start", loc));
        match translate_one(&run, loc, path).await {
            Ok(Some(out_path)) => {
                let mut p = progress(task_id, "file-done", loc);
                p.out_path = Some(out_path);
                run.emit(p);
            }
            Ok(None) => run.emit(progress(task_id, "cancelled", loc)),
            Err(e) => {
                let mut p = progress(task_id, "file-failed", loc);
                p.error = Some(e);
                run.emit(p);
            }
        }
    }
    let mut p = progress(task_id, "queue-finished", Loc::new("", file_count, file_count));
    p.error = run.hub.fatal();
    run.emit(p);
}

async fn run_all_texts(app: &AppHandle, spec: &AiTextsSpec, task_id: u64, cancel: &Arc<AtomicBool>) {
    let hub = Hub::for_job(
        creds_of(&spec.base_url, &spec.api_key, &spec.model),
        spec.model_kind,
        spec.chunk_size,
        spec.concurrency,
        spec.merge_lines,
        Arc::clone(cancel),
    );
    let run = Run::new(app, task_id, hub);
    let loc = Loc::new("", 0, spec.texts.len());
    let translated = translate_chunks(&run, loc, &spec.texts).await;
    match translated {
        Some(list) => {
            let mut p = progress(task_id, "queue-finished", loc);
            p.segment_count = spec.texts.len();
            p.translated = Some(list);
            p.error = run.hub.fatal();
            run.emit(p);
        }
        // 被取消：先说 cancelled，再照常收尾，前端只需盯 queue-finished 收工
        None => {
            run.emit(progress(task_id, "cancelled", loc));
            let mut p = progress(task_id, "queue-finished", loc);
            p.error = run.hub.fatal();
            run.emit(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_false_for_unknown_and_true_only_while_running() {
        // 任务表是全局的：不存在的 id 必须回 false，前端据此判断「已经收尾了」
        assert!(!cancel(u64::MAX));
        let flag = Arc::new(AtomicBool::new(false));
        AI_FLAGS.lock().insert(999_999, Arc::clone(&flag));
        assert!(cancel(999_999));
        assert!(flag.load(Ordering::SeqCst), "取消要落到标志位上");
        AI_FLAGS.lock().remove(&999_999);
        assert!(!cancel(999_999));
    }

    /// 档位字段是可选的：老前端不传 modelKind 时必须照旧跑，不能因为升级了就换参数
    #[test]
    fn specs_stay_compatible_without_the_new_fields() {
        let s: AiTranslateSpec =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","paths":["a.lrc"]}"#).unwrap();
        assert_eq!(s.model_kind, ModelKind::Remote);
        assert_eq!(s.merge_lines, None);
        let s2: AiTextsSpec =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","texts":["x"],"modelKind":"local","mergeLines":6}"#)
                .unwrap();
        assert_eq!((s2.model_kind, s2.merge_lines), (ModelKind::Local, Some(6)));
    }

    /// 收尾事件带 error 是「队列提前结束」的唯一对外信号，缺席时必须整键消失
    #[test]
    fn the_finish_event_only_carries_an_error_when_the_queue_aborted() {
        let mut ok = progress(1, "queue-finished", Loc::new("", 2, 2));
        assert!(serde_json::to_value(&ok).unwrap().get("error").is_none());
        ok.error = Some("请求失败: 连接被拒绝".into());
        assert_eq!(serde_json::to_value(&ok).unwrap()["error"], "请求失败: 连接被拒绝");
    }
}
