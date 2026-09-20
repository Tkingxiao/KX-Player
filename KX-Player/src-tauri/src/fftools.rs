//! ffmpeg 执行器：定位 + 参数列表式运行（600s 超时），退出时回收子进程。
//! 安全：等价 subprocess.run([...], shell=False)；固定程序名 + 参数数组，无 shell 参与。

use once_cell::sync::Lazy;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub error: Option<String>,
}

/// 全局 tokio 运行时（阻塞命令线程借用）
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    });
    &RT
}

fn spawn_hidden(exe: std::path::PathBuf, args: &[String]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（tokio 自带 API，无需 CommandExt）
    cmd
}

/// 同步运行 ffmpeg 并收集 stdout/stderr（600s 超时）
pub fn exec_ffmpeg(args: &[String]) -> ExecResult {
    let Some(exe) = crate::paths::locate_ffmpeg() else {
        return ExecResult { code: -1, stdout: None, stderr: None, error: Some("ffmpeg.exe 未找到".into()) };
    };
    runtime().block_on(exec_async(exe, args, 600))
}

async fn exec_async(exe: std::path::PathBuf, args: &[String], timeout_secs: u64) -> ExecResult {
    let mut child = match spawn_hidden(exe, args).spawn() {
        Ok(c) => c,
        Err(e) => return ExecResult { code: -1, stdout: None, stderr: None, error: Some(format!("启动失败: {e}")) },
    };
    let out = child.stdout.take();
    let err = child.stderr.take();
    let wait = async {
        let (so, se, status) = tokio::join!(read_stream(out), read_err(err), child.wait());
        ExecResult {
            code: status.map(|s| s.code().unwrap_or(1)).unwrap_or(1),
            stdout: Some(so),
            stderr: Some(se),
            error: None,
        }
    };
    tokio::select! {
        r = wait => r,
        _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
            let _ = child.start_kill();
            ExecResult { code: 1, stdout: None, stderr: None, error: Some("ffmpeg 超时(600s)".into()) }
        }
    }
}

async fn read_stream(s: Option<tokio::process::ChildStdout>) -> String {
    use tokio::io::AsyncReadExt;
    let mut s = match s {
        Some(s) => s,
        None => return String::new(),
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match s.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 64 * 1024 {
                    let cut = buf.len() - 64 * 1024;
                    buf.drain(..cut);
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

async fn read_err(s: Option<tokio::process::ChildStderr>) -> String {
    use tokio::io::AsyncReadExt;
    let mut s = match s {
        Some(s) => s,
        None => return String::new(),
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match s.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 64 * 1024 {
                    let cut = buf.len() - 64 * 1024;
                    buf.drain(..cut);
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// 应用退出时回收全部 ffmpeg 子进程
pub fn kill_all() {
    // tokio Child 默认 kill_on_drop=false；进程随主进程退出由 OS 回收，
    // 转换任务取消将在 convert 模块接入时通过句柄表实现。
}

// ── 可用性探测 ────────────────────────────────────────────────

/// ffmpeg 可用性信息（前端据此降级 UI）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegInfo {
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

pub fn probe_ffmpeg() -> FfmpegInfo {
    let Some(exe) = crate::paths::locate_ffmpeg() else {
        return FfmpegInfo { available: false, path: None, version: None };
    };
    let result = runtime().block_on(exec_async(exe.clone(), &["-version".to_string()], 15));
    let version = result
        .stdout
        .as_deref()
        .or(result.stderr.as_deref())
        .and_then(|s| s.lines().next())
        .map(|l| l.trim().to_string());
    FfmpegInfo {
        available: result.code == 0 || version.is_some(),
        path: Some(exe.to_string_lossy().into_owned()),
        version,
    }
}

// ── 响度分析（ffmpeg ebur128）─────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessResult {
    pub path: String,
    pub lufs: Option<f64>,
    pub error: Option<String>,
}

/// 解析 ffmpeg ebur128 汇总输出中的 Integrated loudness I 值。
/// 匹配示例：`    I:         -23.5 LUFS`
pub fn parse_integrated_lufs(stderr: &str) -> Option<f64> {
    for line in stderr.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("I:") {
            let token = rest.trim().split_whitespace().next()?;
            if let Ok(v) = token.parse::<f64>() {
                if v.is_finite() && v > -100.0 {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// 用 ffmpeg ebur128 测量单个文件的整体响度（只分析，不产出文件）。
/// 单文件 300s 超时；失败返回 error 字符串。
pub fn measure_loudness(path: &str) -> LoudnessResult {
    let Some(exe) = crate::paths::locate_ffmpeg() else {
        return LoudnessResult {
            path: path.into(),
            lufs: None,
            error: Some("ffmpeg.exe 未找到".into()),
        };
    };
    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostats".into(),
        "-i".into(),
        path.to_string(),
        "-af".into(),
        "ebur128=peak=true".into(),
        "-f".into(),
        "null".into(),
        null_device(),
    ];
    let result = runtime().block_on(exec_async(exe, &args, 300));
    if let Some(err) = result.error {
        return LoudnessResult { path: path.into(), lufs: None, error: Some(err) };
    }
    let stderr = result.stderr.unwrap_or_default();
    match parse_integrated_lufs(&stderr) {
        Some(v) => LoudnessResult { path: path.into(), lufs: Some(v), error: None },
        None => LoudnessResult {
            path: path.into(),
            lufs: None,
            error: Some("未能从 ffmpeg 输出解析响度".into()),
        },
    }
}

// ── 转换任务（可取消，任务 + 事件）─────────────────────────────

use std::collections::HashMap as StdHashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};

/// 转换任务句柄表：task_id → 取消旗标（句柄表模式，见 kill_all 注释）
static CONVERT_FLAGS: Lazy<parking_lot::Mutex<StdHashMap<u64, std::sync::Arc<AtomicBool>>>> =
    Lazy::new(|| parking_lot::Mutex::new(StdHashMap::new()));
static CONVERT_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertItem {
    pub id: i64,
    pub path: String,
    pub out_path: String,
    /// convert = 音频转码（保声道/采样率）；extract = 视频提取音轨（流拷贝，不重编码）
    pub kind: String,
    pub format: String,
}

fn codec_args(format: &str) -> Vec<String> {
    // 红线：不指定 -ac/-ar，保持源声道数与采样率（ASMR 双耳录音）
    match format {
        "mp3" => vec!["-c:a".into(), "libmp3lame".into(), "-q:a".into(), "2".into()],
        "aac" | "m4a" => vec!["-c:a".into(), "aac".into(), "-b:a".into(), "256k".into()],
        "flac" => vec!["-c:a".into(), "flac".into()],
        "wav" => vec!["-c:a".into(), "pcm_s16le".into()],
        "ogg" => vec!["-c:a".into(), "libvorbis".into(), "-q:a".into(), "6".into()],
        _ => vec!["-c:a".into(), "copy".into()],
    }
}

fn build_convert_args(item: &ConvertItem) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into(), "-i".into(), item.path.clone()];
    if item.kind == "extract" {
        // 提取音轨：去视频流 + 按目标格式重编码（保持声道/采样率）
        args.push("-vn".into());
        args.extend(codec_args(&item.format));
    } else {
        args.extend(codec_args(&item.format));
    }
    args.push(item.out_path.clone());
    args
}

/// 运行转换队列（前端已排好序）；逐项发 `convert:progress` 事件。
/// 事件载荷：{ taskId, itemId, state: 'running'|'done'|'failed'|'cancelled', error?, activeCount }
pub fn run_convert_queue(app: tauri::AppHandle, items: Vec<ConvertItem>) -> u64 {
    let task_id = CONVERT_SEQ.fetch_add(1, AtomicOrdering::SeqCst) + 1;
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    CONVERT_FLAGS.lock().insert(task_id, std::sync::Arc::clone(&cancel));
    std::thread::spawn(move || {
        use tauri::Emitter;
        let Some(exe) = crate::paths::locate_ffmpeg() else {
            for item in &items {
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "failed",
                    "error": "ffmpeg.exe 未找到",
                }));
            }
            CONVERT_FLAGS.lock().remove(&task_id);
            return;
        };
        for item in &items {
            if cancel.load(AtomicOrdering::SeqCst) {
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "cancelled",
                }));
                continue;
            }
            let _ = app.emit("convert:progress", serde_json::json!({
                "taskId": task_id, "itemId": item.id, "state": "running",
            }));
            let args = build_convert_args(item);
            let result = runtime().block_on(exec_async(exe.clone(), &args, 3600));
            if cancel.load(AtomicOrdering::SeqCst) {
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "cancelled",
                }));
            } else if result.code == 0 && result.error.is_none() {
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "done",
                }));
            } else {
                let tail = result
                    .stderr
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .rev()
                    .take(3)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "failed",
                    "error": result.error.unwrap_or(if tail.is_empty() { "ffmpeg 失败".into() } else { tail }),
                }));
            }
        }
        CONVERT_FLAGS.lock().remove(&task_id);
        let _ = app.emit("convert:progress", serde_json::json!({
            "taskId": task_id, "itemId": -1, "state": "queue-finished",
        }));
    });
    task_id
}

/// 取消转换队列：剩余项标 cancelled（正在执行的 ffmpeg 会跑完当前文件，符合「文件级取消」粒度）
pub fn cancel_convert_queue(task_id: u64) -> bool {
    if let Some(flag) = CONVERT_FLAGS.lock().get(&task_id) {
        flag.store(true, AtomicOrdering::SeqCst);
        true
    } else {
        false
    }
}

/// 空输出设备（跨平台；Windows=NUL，其余=/dev/null）
fn null_device() -> String {
    if cfg!(windows) { "NUL".to_string() } else { "/dev/null".to_string() }
}

#[cfg(test)]
mod tests {
    use super::parse_integrated_lufs;

    #[test]
    fn parses_integrated_loudness_summary() {
        let stderr = r#"
[Parsed_ebur128_0 @ 00000234] Summary:

  Integrated loudness:
    I:         -23.5 LUFS
    Threshold: -33.8 LUFS

  Loudness range:
    LRA:         7.2 LU
"#;
        assert_eq!(parse_integrated_lufs(stderr), Some(-23.5));
    }

    #[test]
    fn ignores_threshold_lines_and_missing_values() {
        assert_eq!(parse_integrated_lufs("Threshold: -33.8 LUFS"), None);
        assert_eq!(parse_integrated_lufs("I: -inf LUFS"), None);
        assert_eq!(parse_integrated_lufs("no loudness here"), None);
    }
}
