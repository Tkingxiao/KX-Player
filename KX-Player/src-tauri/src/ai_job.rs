//! AI 批量任务：字幕翻译从「前端逐块 await」改成「Rust 侧任务 + ai:progress 事件」（B3）。
//!
//! 与转换队列（fftools::run_convert_queue）同一套路：任务号 + 取消标志表 + 事件广播。
//! 为什么要搬：旧实现每翻译 15 行就要过一次 IPC（readTextFile → 逐块 aiChat → 用 base64 写回），
//! 一个 300 行的字幕文件是 20+ 次往返，中途关窗口就丢进度、也取消不了；现在整条链路在 Rust
//! 里跑，前端只负责「启动 + 收事件 + 画进度」。
//!
//! 本文件只管「任务表 + 事件流 + 队列循环」，「一批文本怎么翻」在 `ai_chunk.rs`，
//! 「单文件的豁免/缓存/落盘」在 `ai_file.rs`，「外接 / 本地各自的参数档」在 `ai_strategy.rs`，
//! 「事件的载荷结构」在 `ai_event.rs`。两个任务共用同一条 `ai:progress`：
//! - `run_translate`：一批字幕文件 → 各自的 `.zh.*` 译文文件（写盘）
//! - `run_texts`：一批纯文本（AI 页的「文件夹名翻译」）→ 只在收尾事件里回传译文，不落盘

use crate::ai_chunk::{creds_of, translate_chunks, Hub, Run};
use crate::ai_event::{progress, AiProgress, Loc};
use crate::ai_file::translate_one;
use crate::ai_strategy::ModelKind;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap as StdHashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTranslateSpec {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 待翻译的字幕文件路径（扫描由 subtitle_scan_dir 负责，任务只认路径）
    pub paths: Vec<String>,
    /// 点名要**真重译**的路径子集（行内「重译」放进队列的）：对这些文件绕开「已翻译
    /// 豁免」与「内容复用缓存」，逼一次真翻译 —— 否则重译只是把旧译文再抄一遍。
    #[serde(default)]
    pub force_paths: Vec<String>,
    /// 模型档位，缺省 `remote` = 第 11 批的参数
    #[serde(default)]
    pub model_kind: ModelKind,
    /// 接口协议，缺省 `openai-chat` = 既有行为（老前端不带这个字段时照旧）
    #[serde(default)]
    pub api_style: crate::model::ApiStyle,
    /// 本地模型一次合并几条（2–10），只有 `modelKind = local` 时起作用
    pub merge_lines: Option<usize>,
    /// 显式覆盖档位（前端两页都不传，留着给调用方精确控制）
    pub chunk_size: Option<usize>,
    pub concurrency: Option<usize>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTextsSpec {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 待翻译的纯文本，顺序即回传顺序
    pub texts: Vec<String>,
    #[serde(default)]
    pub model_kind: ModelKind,
    /// 接口协议，缺省 `openai-chat` = 既有行为
    #[serde(default)]
    pub api_style: crate::model::ApiStyle,
    pub merge_lines: Option<usize>,
    pub chunk_size: Option<usize>,
    pub concurrency: Option<usize>,
}

/// 任务跑完就把 Key 从内存里擦掉（P0-3）。任务可能跑几十分钟，这两个 spec 会一直
/// 活在线程栈上 —— 不擦的话，Key 会以明文形式留在堆里等下一个分配者。
impl Drop for AiTranslateSpec {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
    }
}

impl Drop for AiTextsSpec {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
    }
}

/// 手写而非 derive：`{:?}` 一份 spec 就是一条带 Key 的日志，输出里只留长度。
impl std::fmt::Debug for AiTranslateSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiTranslateSpec")
            .field("base_url", &self.base_url)
            .field("api_key", &crate::ai::redact(&self.api_key))
            .field("model", &self.model)
            .field("paths", &self.paths.len())
            .field("force_paths", &self.force_paths.len())
            .field("model_kind", &self.model_kind)
            .field("api_style", &self.api_style.name())
            .field("merge_lines", &self.merge_lines)
            .field("chunk_size", &self.chunk_size)
            .field("concurrency", &self.concurrency)
            .finish()
    }
}

impl std::fmt::Debug for AiTextsSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiTextsSpec")
            .field("base_url", &self.base_url)
            .field("api_key", &crate::ai::redact(&self.api_key))
            .field("model", &self.model)
            .field("texts", &self.texts.len())
            .field("model_kind", &self.model_kind)
            .field("api_style", &self.api_style.name())
            .field("merge_lines", &self.merge_lines)
            .field("chunk_size", &self.chunk_size)
            .field("concurrency", &self.concurrency)
            .finish()
    }
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
    // 退出途中不再起新任务（P1-3 第一步「停止接收」）：与「启动被拒」同一口径 ——
    // 没有东西在跑，也就不登记任务表，只把原因送出去。否则这个任务会赶在收尾线程
    // 后面开始写盘，而收尾的「等任务」已经数不到它。
    if !crate::shutdown::accepting() {
        emit_reject(&app, task_id, units, "应用正在退出，未启动任务".to_string());
        return task_id;
    }
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

/// 退出用（P1-3）：取消所有在飞任务（返回条数）。与 `cancel` 同一粒度 ——
/// 正在飞的那一块跑完、后续不再开始，任务自己走到收尾。
pub fn cancel_all() -> usize {
    let flags: Vec<Arc<AtomicBool>> = AI_FLAGS.lock().values().map(Arc::clone).collect();
    for flag in &flags {
        flag.store(true, Ordering::SeqCst);
    }
    flags.len()
}

/// 在飞任务条数（退出收尾等它归零的判据）。
pub fn running() -> usize {
    AI_FLAGS.lock().len()
}

/// 命令层在开跑前就拦下的启动（路径越界等）：**不登记任务表**（没有东西在跑，
/// `cancel` 也不该认它），只把原因用与正常收尾同构的一条事件送达 ——
/// `ai_translate_start` 的返回类型是 taskId、回不了错误信封，前端也就少一个分支。
pub fn reject(app: AppHandle, units: usize, error: String) -> u64 {
    let task_id = AI_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    emit_reject(&app, task_id, units, error);
    task_id
}

fn emit_reject(app: &AppHandle, task_id: u64, units: usize, error: String) {
    if let Err(e) = app.emit("ai:progress", rejected_event(task_id, units, error)) {
        crate::paths::append_log(&format!("[ai] 启动被拒且事件也投不出去: {e}"));
    }
}

/// 拒绝事件的口径与 `run_all` 的提前退出完全一致：`queue-finished` 带 error，
/// 位置字段是「整批的队首」（`file_index = 0`、`file_count = 条数`）。
fn rejected_event(task_id: u64, units: usize, error: String) -> AiProgress {
    let mut p = progress(task_id, "queue-finished", Loc::new("", 0, units));
    p.error = Some(error);
    p
}

/// 流水线预备深度：同时处于「已读入并切好块」状态的文件数。
/// 前一个文件的块还在全局池里飞时，后面的文件就提前读盘、解析、把块排进池子 ——
/// 「上一份的最后几块 + 下一份还没开始切」的文件边界不再让并发空转。
/// 深度再大只是多占内存（每多一份解析结果），4 份足够喂满默认并发。
const PREPARE_AHEAD: usize = 4;

async fn run_all(app: &AppHandle, spec: &AiTranslateSpec, task_id: u64, cancel: &Arc<AtomicBool>) {
    let run = Arc::new(Run::new(
        app.clone(),
        task_id,
        Hub::for_job(
            creds_of(&spec.base_url, &spec.api_key, &spec.model, spec.api_style),
            spec.model_kind,
            spec.chunk_size,
            spec.concurrency,
            spec.merge_lines,
            Arc::clone(cancel),
        ),
    ));
    let file_count = spec.paths.len();
    // 端点地址不合规（P0-3）：一条都别发 —— 否则用户看到的是几条「已取消」，
    // 真正的原因藏在收尾事件里
    if let Err(e) = run.hub.check_ready() {
        let mut p = progress(task_id, "queue-finished", Loc::new("", 0, file_count));
        p.error = Some(e);
        run.emit(p);
        return;
    }
    let prepare = Arc::new(tokio::sync::Semaphore::new(PREPARE_AHEAD));
    let mut files = tokio::task::JoinSet::new();
    for (i, path) in spec.paths.iter().enumerate() {
        // 行内「重译」点名的那几个文件要逼一次真翻译：绕开豁免账与内容缓存（ai_file）
        let force = spec.force_paths.iter().any(|p| p == path);
        files.spawn(file_task(
            Arc::clone(&run),
            path.clone(),
            i,
            file_count,
            force,
            Arc::clone(&prepare),
        ));
    }
    while files.join_next().await.is_some() {}
    let mut p = progress(task_id, "queue-finished", Loc::new("", file_count, file_count));
    p.error = run.hub.fatal();
    run.emit(p);
}

/// 一个文件的全生命周期：排队拿「预备深度」名额 → file-start → 翻译（块在全局池里飞，
/// 同期后面文件的块也在这个池里排队）→ 写盘记账 → 终态事件。
/// 取消语义与旧版一致：没开始就 cancelled；翻到一半的在飞块跑完为止，不写盘。
async fn file_task(
    run: Arc<Run>,
    path: String,
    index: usize,
    count: usize,
    force: bool,
    prepare: Arc<tokio::sync::Semaphore>,
) {
    // 预备名额先于一切：没排到就不发 file-start，界面上它还是「等待」
    let Ok(_permit) = prepare.acquire().await else { return };
    let loc = Loc::new(&path, index, count);
    if run.hub.cancel.load(Ordering::SeqCst) {
        run.emit(progress(run.task_id, "cancelled", loc));
        return;
    }
    run.emit(progress(run.task_id, "file-start", loc));
    match translate_one(&run, loc, &path, force).await {
        Ok(Some(done)) => {
            let mut p = progress(run.task_id, "file-done", loc);
            p.out_path = Some(done.out_path);
            p.segment_count = done.segment_count;
            if done.fallback_lines > 0 {
                p.fallback_lines = Some(done.fallback_lines);
            }
            run.emit(p);
        }
        Ok(None) => run.emit(progress(run.task_id, "cancelled", loc)),
        Err(e) => {
            let mut p = progress(run.task_id, "file-failed", loc);
            p.error = Some(e);
            run.emit(p);
        }
    }
}

async fn run_all_texts(app: &AppHandle, spec: &AiTextsSpec, task_id: u64, cancel: &Arc<AtomicBool>) {
    let hub = Hub::for_job(
        creds_of(&spec.base_url, &spec.api_key, &spec.model, spec.api_style),
        spec.model_kind,
        spec.chunk_size,
        spec.concurrency,
        spec.merge_lines,
        Arc::clone(cancel),
    );
    let run = Run::new(app.clone(), task_id, hub);
    let loc = Loc::new("", 0, spec.texts.len());
    if let Err(e) = run.hub.check_ready() {
        let mut p = progress(task_id, "queue-finished", loc);
        p.error = Some(e);
        run.emit(p);
        return;
    }
    let translated = translate_chunks(&run, loc, &spec.texts).await;
    match translated {
        Some((list, fallback)) => {
            let mut p = progress(task_id, "queue-finished", loc);
            p.segment_count = spec.texts.len();
            p.translated = Some(list);
            if fallback > 0 {
                p.fallback_lines = Some(fallback);
            }
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
        assert_eq!(s.api_style, crate::model::ApiStyle::OpenAiChat, "apiStyle 缺席 = 照旧走 chat/completions");
        assert_eq!(s.merge_lines, None);
        assert!(s.force_paths.is_empty(), "forcePaths 缺席 = 全部走省钱闸门，不逼重译");
        let s2: AiTextsSpec =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","texts":["x"],"modelKind":"local","mergeLines":6}"#)
                .unwrap();
        assert_eq!((s2.model_kind, s2.merge_lines), (ModelKind::Local, Some(6)));
        let s3: AiTranslateSpec = serde_json::from_str(
            r#"{"baseUrl":"u","apiKey":"k","model":"m","paths":["a.lrc"],"forcePaths":["a.lrc"]}"#,
        )
        .unwrap();
        assert_eq!(s3.force_paths, vec!["a.lrc".to_string()], "行内重译点名走 force 通道");
        let s4: AiTranslateSpec = serde_json::from_str(
            r#"{"baseUrl":"u","apiKey":"k","model":"m","paths":["a.lrc"],"apiStyle":"anthropic"}"#,
        )
        .unwrap();
        assert_eq!(s4.api_style, crate::model::ApiStyle::Anthropic, "新协议随 spec 透传到批量任务");
    }

    /// 收尾事件带 error 是「队列提前结束」的唯一对外信号，缺席时必须整键消失
    #[test]
    fn the_finish_event_only_carries_an_error_when_the_queue_aborted() {
        let mut ok = progress(1, "queue-finished", Loc::new("", 2, 2));
        assert!(serde_json::to_value(&ok).unwrap().get("error").is_none());
        ok.error = Some("请求失败: 连接被拒绝".into());
        assert_eq!(serde_json::to_value(&ok).unwrap()["error"], "请求失败: 连接被拒绝");
    }

    /// 启动被拒（路径越界）走的是与收尾同形的一条事件：前端只认 `queue-finished` 收工，
    /// 位置字段是整批队首，`fileCount` 仍是这批的条数（界面要显示「这一批几行」）。
    #[test]
    fn a_rejected_start_still_finishes_the_queue_with_the_reason() {
        let p = rejected_event(11, 3, "路径不在已授权的范围内：x.lrc（不在任何已授权目录里）".into());
        assert_eq!((p.task_id, p.state.as_str(), p.file_index, p.file_count), (11, "queue-finished", 0, 3));
        assert_eq!(p.error.as_deref(), Some("路径不在已授权的范围内：x.lrc（不在任何已授权目录里）"));
        assert!(p.translated.is_none(), "被拒的启动不带译文");
    }

    /// 退出收尾（P1-3）靠这两个方法判断「AI 真的停了」——口径与 `cancel` 一致：
    /// 只认登记过的任务，没登记过的 id 不认（它本来就没在跑）。
    #[test]
    fn cancel_all_marks_every_registered_job_and_running_counts_them() {
        let before = running();
        let a = Arc::new(AtomicBool::new(false));
        let b = Arc::new(AtomicBool::new(false));
        AI_FLAGS.lock().insert(9_000_001, Arc::clone(&a));
        AI_FLAGS.lock().insert(9_000_002, Arc::clone(&b));
        assert_eq!(running(), before + 2, "在飞条数就是任务表的大小");
        assert_eq!(cancel_all(), before + 2);
        assert!(a.load(Ordering::SeqCst) && b.load(Ordering::SeqCst), "登记过的都要置取消位");
        assert!(!cancel(9_000_003), "没登记过的 id 不认");
        AI_FLAGS.lock().remove(&9_000_001);
        AI_FLAGS.lock().remove(&9_000_002);
        assert_eq!(running(), before, "用例收尾要把表还原");
    }
}
