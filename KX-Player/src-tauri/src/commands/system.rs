//! 命令层：设置/背景图/窗口/ffmpeg/AI。

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::{AiChatPayload, AiChatResult, AiModelsResult, AiPingResult, BgImageData};
use serde_json::Value;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use super::join_task;
use crate::scope::PathScope;

// ── 设置 ────────────────────────────────────────────────────────

#[tauri::command]
pub fn load_settings() -> Value {
    crate::settingsio::load(&crate::paths::settings_path())
}

#[tauri::command]
pub fn save_settings(settings: Value) -> bool {
    crate::settingsio::save(&crate::paths::settings_path(), &settings)
}

// ── 背景图 ──────────────────────────────────────────────────────

#[tauri::command]
pub fn load_bg_image() -> Option<BgImageData> {
    crate::bgimage::load()
}

#[tauri::command]
pub fn save_bg_image(data_url: String) -> bool {
    crate::bgimage::save_data_url(&data_url)
}

#[tauri::command]
pub fn remove_bg_image() -> bool {
    crate::bgimage::remove()
}

// ── 启动自检 ────────────────────────────────────────────────────

/// 取走 setup 阶段收集的降级提示；取一次即清空，重启才会再报。
#[tauri::command]
pub fn startup_warnings() -> Vec<String> {
    crate::state::take_startup_warnings()
}

// ── 窗口 ────────────────────────────────────────────────────────

#[tauri::command]
pub fn minimize_window(window: WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
pub fn maximize_window(window: WebviewWindow) {
    match window.is_maximized() {
        Ok(true) => {
            let _ = window.unmaximize();
        }
        _ => {
            let _ = window.maximize();
        }
    }
}

#[tauri::command]
pub fn close_window(window: WebviewWindow) {
    // 关闭 = 隐藏到托盘
    let _ = window.hide();
}

#[tauri::command]
pub fn is_maximized(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

#[tauri::command]
pub fn force_close_window(app: AppHandle) {
    // 与托盘「退出」同一条路（P1-3）：不再「置位 → 睡 250ms → 关窗」——
    // 关窗之后还有 watcher/AI/子进程/写盘要收，那些都是收尾流水线的事。
    crate::shutdown::request_exit(&app, "force_close_window");
}

/// 前端落盘回执（P1-3 第六步）：`App.vue` 在 `save_settings` 真的返回之后调用，
/// 退出流程据此确认「设置已经写进盘」而不是睡一会儿再假设它写完了。
#[tauri::command]
pub fn shutdown_ready(ok: bool) {
    crate::shutdown::mark_ready(ok);
}

#[tauri::command]
pub fn toggle_fullscreen(window: WebviewWindow) -> IpcResult<bool> {
    let q = |e: tauri::Error| IpcError::with_detail(AppErrorCode::Internal, "切换全屏失败", e);
    let on = !window.is_fullscreen().map_err(q)?;
    window.set_fullscreen(on).map_err(q)?;
    Ok(on)
}

// ── ffmpeg ──────────────────────────────────────────────────────

/// ffmpeg 可用性探测（转换页据此降级）
#[tauri::command]
pub fn ffmpeg_probe() -> crate::fftools::FfmpegInfo {
    crate::fftools::probe_ffmpeg()
}

/// 启动转换队列（任务 + convert:progress 事件），返回 taskId。
/// 通用执行器 `ffmpeg_exec` 已删除（P0-2）：前端只能走这条白名单通道，
/// 校验不过整批拒，不会把一个任意 argv 交给 ffmpeg。
#[tauri::command]
pub fn convert_run(app: AppHandle, scope: State<'_, crate::scope::PathScope>, items: Vec<crate::fftools::ConvertItem>) -> IpcResult<u64> {
    // 退出途中不再开转换队列（P1-3 第一步「停止接收」）：队列一起来就会赶在收尾的
    // 「等任务」之后，而那时已经没人等它了
    if !crate::shutdown::accepting() {
        return Err(IpcError::new(AppErrorCode::Busy, "应用正在退出，未开始转换"));
    }
    match crate::fftools::validate_items(&scope, &items) {
        Ok(()) => Ok(crate::fftools::run_convert_queue(app, items)),
        Err(crate::fftools::Reject::Outside(p)) => {
            crate::paths::append_log(&format!("[scope] convert_run 拒绝整批：{p}"));
            Err(IpcError::outside_scope(&p, "输入不在已授权路径内"))
        }
        Err(crate::fftools::Reject::Invalid(m)) => Err(IpcError::invalid_argument(m)),
    }
}

/// 取消转换队列（幂等）
#[tauri::command]
pub fn convert_cancel(task_id: u64) -> bool {
    crate::fftools::cancel_convert_queue(task_id)
}

// ── AI ──────────────────────────────────────────────────────────

/// 单次补全。**async 不是风格选择**：同步命令跑在主线程，120 s 的超时会把整个 UI 冻住，
/// 而且用户点不了取消（B3）。批量翻译走 `ai_translate_start` 的任务 + 事件通道。
#[tauri::command]
pub async fn ai_chat(payload: AiChatPayload) -> AiChatResult {
    if payload.messages.is_empty() {
        return AiChatResult { ok: false, content: String::new(), error: Some("参数错误".into()) };
    }
    match crate::ai::chat_completion(&payload, 120_000).await {
        Ok(content) => AiChatResult { ok: true, content, error: None },
        Err(e) => AiChatResult { ok: false, content: String::new(), error: Some(e.text()) },
    }
}

#[tauri::command]
pub async fn ai_ping(payload: AiChatPayload) -> AiPingResult {
    crate::ai::ping(&payload).await
}

#[tauri::command]
pub async fn ai_list_models(base_url: String, api_key: String) -> AiModelsResult {
    // Key 从进来到用完只有这一份：Tauri 交过来的 String 用完就擦（P0-3）
    let api_key = zeroize::Zeroizing::new(api_key);
    crate::ai::list_models(&base_url, api_key.as_str()).await
}

/// AI 命令的路径闸门（P0-1 的 AI 侧）：与 `rename` / `convert` 同口径 —— 任一条不在授权范围内
/// 就**整批拒**（要么全做要么不做）。返回「哪一条 + 为什么」，文案由调用方组装：能回错误信封的
/// 用 `IpcError::outside_scope`，`ai_translate_start` 回不了（它的返回类型是 taskId），
/// 改用收尾事件的 `error` 送达。合法来路只有一条：路径出自 `open_folder` 选中的目录
/// （`subtitle_scan_dir` 扫出来的），所以正常流程走不到这里 —— 这是道边界，不是默认路径。
fn scope_denial(scope: &PathScope, cmd: &str, paths: &[String]) -> Option<(String, String)> {
    match scope.check_all(paths) {
        Ok(()) => None,
        Err((bad, why)) => {
            crate::paths::append_log(&format!("[scope] {cmd} 拒绝整批：{bad}（{why:?}）"));
            Some((bad, why.to_string()))
        }
    }
}

/// 启动字幕翻译任务（返回 taskId；进度与结果走 ai:progress 事件，见 B3）
#[tauri::command]
pub fn ai_translate_start(
    app: AppHandle,
    scope: State<'_, crate::scope::PathScope>,
    spec: crate::ai_job::AiTranslateSpec,
) -> u64 {
    if let Some((bad, why)) = scope_denial(&scope, "ai_translate_start", &spec.paths) {
        return crate::ai_job::reject(app, spec.paths.len(), format!("路径不在已授权的范围内：{bad}（{why}）"));
    }
    crate::ai_job::run_translate(app, spec)
}

/// 取消翻译任务（幂等；块级粒度：在飞的那一块跑完就停）
#[tauri::command]
pub fn ai_task_cancel(task_id: u64) -> bool {
    crate::ai_job::cancel(task_id)
}

/// 启动纯文本批量翻译任务（AI 页的「文件夹名翻译」）。
/// 与 `ai_translate_start` 同一条 `ai:progress` 事件流，收尾事件带 `translated`。
#[tauri::command]
pub fn ai_texts_start(app: AppHandle, spec: crate::ai_job::AiTextsSpec) -> u64 {
    crate::ai_job::run_texts(app, spec)
}

/// AI-16 导出：把选中的字幕切成 `### 序号 ###` 批次写成 prompts.txt，位置由用户在保存框里定。
/// 整条链路不碰 Provider（`05` 第 16 条要的兜底路径），参数只有路径列表。
/// 保存框被取消回 `CANCELLED`（前端据此不出 Toast），写盘失败才回 `IO_FAILED`。
#[tauri::command]
pub async fn ai_export_prompts(
    app: AppHandle,
    scope: State<'_, crate::scope::PathScope>,
    paths: Vec<String>,
) -> IpcResult<crate::ai_export::ExportResult> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中的字幕文件"));
    }
    if let Some((bad, why)) = scope_denial(&scope, "ai_export_prompts", &paths) {
        return Err(IpcError::outside_scope(&bad, why));
    }
    join_task(
        "导出翻译批次",
        tauri::async_runtime::spawn_blocking(move || {
            let loaded = crate::ai_export::load(&paths);
            if loaded.files.is_empty() {
                return Err(IpcError::invalid_argument("这些文件里没有可导出的字幕文本"));
            }
            let content = crate::ai_export::build_prompt(&loaded);
            if content.is_empty() {
                // 全批没有一条能安全切块 —— 开保存框只会得到一个空文件
                return Err(IpcError::invalid_argument("每条原文都自带 `### 数字 ###` 形状的行，无法切块导出"));
            }
            let Some(picked) = app
                .dialog()
                .file()
                .set_title("导出待翻译批次")
                .set_file_name("prompts.txt")
                .add_filter("文本", &["txt"])
                .blocking_save_file()
            else {
                return Err(IpcError::new(AppErrorCode::Cancelled, "已取消导出"));
            };
            let path = picked.to_string();
            if let Err(e) = crate::atomic::write_str(std::path::Path::new(&path), &content) {
                return Err(IpcError::with_detail(AppErrorCode::IoFailed, "写入批次文件失败", e));
            }
            Ok(crate::ai_export::export_meta(&loaded, &path))
        })
        .await,
    )
}

/// AI-16 导入：把手工跑出的译文并回选中的字幕文件。`apply = false` 只做校验和差异预览、
/// 一点不碰磁盘 —— 预览与落地是同一个函数，所以「报可以写几篇」和「实际写了几篇」不会脱节。
/// 译文文本由前端读（`read_text_file` 管编码），这里不再开第二次对话框。
#[tauri::command]
pub async fn ai_import_translations(
    scope: State<'_, crate::scope::PathScope>,
    paths: Vec<String>,
    text: String,
    apply: bool,
) -> IpcResult<crate::ai_export::ImportResult> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中的字幕文件"));
    }
    if text.trim().is_empty() {
        return Err(IpcError::invalid_argument("译文是空的"));
    }
    if let Some((bad, why)) = scope_denial(&scope, "ai_import_translations", &paths) {
        return Err(IpcError::outside_scope(&bad, why));
    }
    join_task(
        "导入翻译批次",
        tauri::async_runtime::spawn_blocking(move || {
            let loaded = crate::ai_export::load(&paths);
            if loaded.files.is_empty() {
                return Err(IpcError::invalid_argument("这些文件里没有可写入的字幕文本"));
            }
            Ok(crate::ai_export::import(&loaded, &text, apply))
        })
        .await,
    )
}

/// 递归扫描字幕文件。替代前端的 api.listDir 递归：
/// 旧实现每层一次 IPC，一个音乐库目录树要几百次往返，且顺序随 readdir 漂移。
/// async + spawn_blocking：整棵树的 readdir 是 IO，同步命令会在主线程跑（见 `commands/library.rs` 的口径）。
#[tauri::command]
pub async fn subtitle_scan_dir(
    app: AppHandle,
    dir: String,
    max_depth: Option<usize>,
    limit: Option<usize>,
) -> Vec<crate::ai_scan::ScannedItem> {
    // 扫描整批拒（与 `scan_folders` 同口径）：回不了错误信封，拒绝就是空列表（原因在 `[scope]` 日志里）。
    // scope 从 `AppHandle` 取而不是当入参：Tauri 不允许「带引用的 async 命令返回非 Result」，
    // 而且这里要在 `spawn_blocking().await` 之前就把借用放掉。
    let denied = {
        let scope = app.state::<PathScope>();
        scope_denial(&scope, "subtitle_scan_dir", std::slice::from_ref(&dir)).is_some()
    };
    if denied {
        return vec![];
    }
    // 深度与数量的缺省值来自旧前端那次递归：字幕 4/500
    let max_depth = max_depth.unwrap_or(4);
    let limit = limit.unwrap_or(500);
    let db = crate::paths::library_db_path();
    match tauri::async_runtime::spawn_blocking(move || {
        let mut items = crate::ai_scan::scan_dir(&dir, max_depth, limit);
        // 回填「已翻译可豁免」：上级账命中且译文文件还在。缓存读失败不阻断扫描（默认全部未翻译）。
        let _ = crate::db::with_db_text(&db, |c| {
            for i in items.iter_mut() {
                i.translated = crate::ai_cache::subtitle_cached(c, &i.path);
            }
            Ok(())
        });
        items
    })
    .await
    {
        Ok(items) => items,
        Err(e) => {
            crate::paths::append_log(&format!("[ai] subtitle_scan_dir 阻塞任务失败: {e}"));
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AI 命令共用的闸门：一条越界整批拒，并且要**指出是哪一条** ——
    /// 用户侧只能看到「这批没跑」，日志与错误文案是唯一能定位到那个文件的地方。
    #[test]
    fn the_ai_scope_gate_refuses_the_whole_batch_and_names_the_offender() {
        let root = std::env::temp_dir().join(format!("kx-ai-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("music")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let inside = root.join("music").join("a.lrc");
        let outside = root.join("other").join("b.lrc");
        std::fs::write(&inside, "[00:01.00]x").unwrap();
        std::fs::write(&outside, "[00:01.00]x").unwrap();

        let scope = PathScope::new();
        let ok = vec![inside.to_string_lossy().to_string()];
        let mixed = vec![ok[0].clone(), outside.to_string_lossy().to_string()];

        let (bad, _) = scope_denial(&scope, "t", &ok).expect("空范围里什么都该拒");
        assert!(bad.ends_with("a.lrc"), "{bad}");
        assert!(scope.allow_root(&root.join("music")));
        assert!(scope_denial(&scope, "t", &ok).is_none(), "授权目录里的文件必须放行");
        let (bad, why) = scope_denial(&scope, "t", &mixed).expect("混进一条越界就得整批拒");
        assert!(bad.ends_with("b.lrc"), "要指出是哪一条：{bad}");
        assert!(why.contains("不在任何已授权目录里"), "{why}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
