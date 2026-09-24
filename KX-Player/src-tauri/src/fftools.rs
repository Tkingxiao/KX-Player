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

/// 全局 tokio 运行时（阻塞命令线程借用）。多线程建不出来就退到单线程；两者都失败返回 Err，
/// 由调用方转成可报告的任务错误，而不是在懒初始化里 panic 掉宿主线程。
pub fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: Lazy<Option<tokio::runtime::Runtime>> = Lazy::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .or_else(|_| tokio::runtime::Builder::new_current_thread().enable_all().build())
            .ok()
    });
    RT.as_ref()
        .ok_or_else(|| "运行时不可用：无法创建 tokio 线程".to_string())
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
    /// 来路（P1-12）：环境变量 / 程序目录 / 常见安装位置 / PATH
    pub source: Option<crate::paths::FfmpegSource>,
    /// 来路不在应用掌控中时的提示（转换页显示一行）；`None` = 不必提示
    pub warn: Option<String>,
}

/// 目录是否在当前用户可写范围里。判不了就当不可写：宁可少提示，不虚报。
fn in_user_writable_dir(dir: &std::path::Path) -> bool {
    let mut homes: Vec<std::path::PathBuf> = Vec::new();
    for key in ["USERPROFILE", "HOME", "LOCALAPPDATA", "APPDATA", "TEMP"] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                homes.push(std::path::PathBuf::from(v));
            }
        }
    }
    homes.push(std::env::temp_dir());
    homes.into_iter().any(|h| dir.starts_with(h))
}

/// P1-12 的可见那一半：来路不在应用掌控里、或文件躺在用户可写目录时给一句提示。
fn source_warning(exe: &std::path::Path, source: crate::paths::FfmpegSource) -> Option<String> {
    use crate::paths::FfmpegSource as S;
    match source {
        S::Env => return Some("由环境变量 KX_FFMPEG 指定，替换不受应用控制".into()),
        S::Path => return Some("从 PATH 找到：同一台机器上别的程序可以先放一个同名文件".into()),
        S::AppDir | S::Fixed => {}
    }
    let dir = exe.parent()?;
    if in_user_writable_dir(dir) {
        return Some(format!("位于当前用户可写的目录（{}），其他程序可以替换它", dir.display()));
    }
    None
}

pub fn probe_ffmpeg() -> FfmpegInfo {
    let Some((exe, source)) = crate::paths::locate_ffmpeg_ex() else {
        return FfmpegInfo { available: false, path: None, version: None, source: None, warn: None };
    };
    let result = match runtime() {
        Ok(rt) => rt.block_on(exec_async(exe.clone(), &["-version".to_string()], 15)),
        Err(e) => {
            return FfmpegInfo {
                available: false,
                path: Some(e.to_string()),
                version: None,
                source: Some(source),
                warn: None,
            };
        }
    };
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
        source: Some(source),
        warn: source_warning(&exe, source),
    }
}

/// 文件指纹（blake3 前 16 个 hex 位，流式读，不整份进内存）。读不动返回 `None`。
fn fingerprint(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
            }
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().to_hex().to_string()[..16].to_string())
}

/// 启动时往日志记一条「用的是哪个 ffmpeg」：实际路径、来路、版本、指纹。
/// 发布版有了这条，用户报「转换失败」时能先对齐二进制是不是同一个（P1-12）。
/// 版本探测要起一个子进程，所以放后台线程，不挡启动。
pub fn log_ffmpeg_identity() {
    std::thread::spawn(|| {
        let Some((exe, source)) = crate::paths::locate_ffmpeg_ex() else {
            crate::paths::append_log("[ffmpeg] 未找到（转换/响度/视频探测不可用）");
            return;
        };
        let version = runtime()
            .ok()
            .and_then(|rt| rt.block_on(exec_async(exe.clone(), &["-version".to_string()], 15)).stdout)
            .and_then(|s| s.lines().next().map(|l| l.trim().to_string()));
        let fp = fingerprint(&exe).unwrap_or_else(|| "指纹不可读".into());
        let warn = source_warning(&exe, source).map(|w| format!("（注意：{w}）")).unwrap_or_default();
        crate::paths::append_log(&format!(
            "[ffmpeg] 使用 {}（来源：{}）{}{warn} 指纹 {fp}",
            exe.display(),
            source.label(),
            version.unwrap_or_else(|| "版本未知".into()),
        ));
    });
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
            let token = rest.split_whitespace().next()?;
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
    let result = match runtime() {
        Ok(rt) => rt.block_on(exec_async(exe, &args, 300)),
        Err(e) => return LoudnessResult { path: path.into(), lufs: None, error: Some(e) },
    };
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
    /// convert = 音频转码（保声道/采样率）；extract = 视频提取音轨
    pub kind: String,
    pub format: String,
}

/// 允许的输出容器，**白名单**。旧实现的 `_ => copy` 分支会让表外格式「成功」出一个
/// 没转码的文件；表外一律拒绝，不再有默认分支。
pub const CONVERT_FORMATS: [&str; 6] = ["mp3", "aac", "m4a", "flac", "wav", "ogg"];

/// 输入扩展名白名单，与转换页两个模式一一对应（前端只做提示，判在这里）。
const AUDIO_EXTS: [&str; 11] = ["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "ape", "wv", "aiff"];
const VIDEO_EXTS: [&str; 7] = ["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv"];

/// 拒绝理由：命令层据此选错误码（越界 vs 参数不合法）。
#[derive(Debug, PartialEq, Eq)]
pub enum Reject {
    Outside(String),
    Invalid(String),
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reject::Outside(p) => write!(f, "{p}"),
            Reject::Invalid(m) => f.write_str(m),
        }
    }
}

fn ext_lower(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// 同一目录（Windows 路径大小写不敏感；两侧都按 `/` 归一后再比）。
fn same_dir(a: &str, b: &str) -> bool {
    let dir = |p: &str| {
        let q = p.replace('\\', "/");
        q.rsplit_once('/').map(|(d, _)| d.to_ascii_lowercase()).unwrap_or_default()
    };
    !a.is_empty() && dir(a) == dir(b)
}

/// 转换项校验（P0-2）：这一层出去之后，`run_convert_queue` 里的每个参数都来自固定 argv 表。
/// - 输入必须在已授权路径内、确实存在、扩展名与模式匹配；
/// - 输出必须与源文件同目录（产品规则：输出到源目录）、扩展名与格式一致、**已存在就拒**；
/// - 表外 `kind`/`format` 直接拒（不再有「未知格式 → 音频流拷贝」的兜底）。
///
/// 粒度是**整批拒**：队列是一批写盘动作，半批放行会让「哪几条真被写了」说不清。
pub fn validate_items(scope: &crate::scope::PathScope, items: &[ConvertItem]) -> Result<(), Reject> {
    if items.is_empty() {
        return Err(Reject::Invalid("没有待转换的文件".into()));
    }
    for item in items {
        let kind_video = match item.kind.as_str() {
            "convert" => false,
            "extract" => true,
            other => return Err(Reject::Invalid(format!("未知的转换类型: {other}"))),
        };
        if !CONVERT_FORMATS.contains(&item.format.as_str()) {
            return Err(Reject::Invalid(format!("不支持的输出格式: {}", item.format)));
        }
        // URL/协议前缀与「看着像选项」的路径在这里就断掉：ffmpeg 的输入位置接受它们
        if item.path.contains("://") || item.path.starts_with('-') {
            return Err(Reject::Invalid(format!("不是本地文件路径: {}", item.path)));
        }
        if !scope.allows(&item.path) {
            return Err(Reject::Outside(item.path.clone()));
        }
        if !std::path::Path::new(&item.path).is_file() {
            return Err(Reject::Invalid(format!("找不到文件: {}", item.path)));
        }
        let ext = ext_lower(&item.path);
        let ok_ext = if kind_video { VIDEO_EXTS.contains(&ext.as_str()) } else { AUDIO_EXTS.contains(&ext.as_str()) };
        if !ok_ext {
            let what = if kind_video { "视频" } else { "音频" };
            return Err(Reject::Invalid(format!("不是{what}文件: {}", item.path)));
        }
        if item.out_path.is_empty() {
            return Err(Reject::Invalid(format!("没有指定输出路径: {}", item.path)));
        }
        if !same_dir(&item.out_path, &item.path) {
            return Err(Reject::Invalid(format!("输出必须与源文件同目录: {}", item.out_path)));
        }
        if item.out_path.eq_ignore_ascii_case(&item.path) {
            return Err(Reject::Invalid(format!("输出不能覆盖源文件: {}", item.out_path)));
        }
        if ext_lower(&item.out_path) != item.format {
            return Err(Reject::Invalid(format!("输出扩展名与格式不符: {}", item.out_path)));
        }
        if std::path::Path::new(&item.out_path).exists() {
            return Err(Reject::Invalid(format!("目标已存在，不覆盖: {}", item.out_path)));
        }
    }
    Ok(())
}

/// 输出格式 → 编码参数。`None` = 表外格式（校验层已拦，这里是第二道）。
fn codec_args(format: &str) -> Option<Vec<String>> {
    // 红线：不指定 -ac/-ar，保持源声道数与采样率（ASMR 双耳录音）
    let args: Vec<&str> = match format {
        "mp3" => vec!["-c:a", "libmp3lame", "-q:a", "2"],
        "aac" | "m4a" => vec!["-c:a", "aac", "-b:a", "256k"],
        "flac" => vec!["-c:a", "flac"],
        "wav" => vec!["-c:a", "pcm_s16le"],
        "ogg" => vec!["-c:a", "libvorbis", "-q:a", "6"],
        _ => return None,
    };
    Some(args.into_iter().map(str::to_string).collect())
}

/// `-n` 而不是 `-y`：即使前面「目标不存在」的检查被并发抢先，也只失败不覆盖。
fn build_convert_args(item: &ConvertItem) -> Option<Vec<String>> {
    let mut args: Vec<String> =
        vec!["-hide_banner".into(), "-nostdin".into(), "-n".into(), "-i".into(), item.path.clone()];
    if item.kind == "extract" {
        // 提取音轨：去视频流 + 按目标格式重编码（保持声道/采样率）
        args.push("-vn".into());
    }
    args.extend(codec_args(&item.format)?);
    args.push(item.out_path.clone());
    Some(args)
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
            let Some(args) = build_convert_args(item) else {
                let _ = app.emit("convert:progress", serde_json::json!({
                    "taskId": task_id, "itemId": item.id, "state": "failed",
                    "error": format!("不支持的输出格式: {}", item.format),
                }));
                continue;
            };
            let result = match runtime() {
                Ok(rt) => rt.block_on(exec_async(exe.clone(), &args, 3600)),
                Err(e) => ExecResult { code: -1, stdout: None, stderr: None, error: Some(e) },
            };
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

    // ── 转换项校验（P0-2）────────────────────────────────────────

    use super::{build_convert_args, validate_items, ConvertItem, Reject};
    use crate::scope::PathScope;

    /// 独立临时目录树（与 scope.rs 用例同一套路，用例之间不共享路径）
    struct Tree(std::path::PathBuf);

    impl Tree {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let root = std::env::temp_dir().join(format!("kx-convert-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&root).expect("建临时目录");
            Tree(root)
        }

        fn file(&self, rel: &str) -> String {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).expect("建父目录");
            }
            std::fs::write(&p, b"x").expect("写文件");
            p.to_string_lossy().into_owned()
        }

        fn path(&self, rel: &str) -> String {
            self.0.join(rel).to_string_lossy().into_owned()
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scope_for(t: &Tree) -> PathScope {
        let scope = PathScope::new();
        assert!(scope.allow_root(&t.0));
        scope
    }

    fn item(path: &str, out: &str, kind: &str, format: &str) -> ConvertItem {
        ConvertItem { id: 1, path: path.into(), out_path: out.into(), kind: kind.into(), format: format.into() }
    }

    /// 一个合法音频项（`base` 目录下的 `a.flac` → `a.mp3`）
    fn good_pair(t: &Tree) -> ConvertItem {
        let src = t.file("base/a.flac");
        let out = t.path("base/a.mp3");
        item(&src, &out, "convert", "mp3")
    }

    #[test]
    fn unknown_or_out_of_whitelist_format_is_rejected() {
        let t = Tree::new("fmt");
        let src = t.file("base/a.flac");
        for fmt in ["avi", "copy", "", "MP3"] {
            let mut it = good_pair(&t);
            it.path = src.clone();
            it.format = fmt.into();
            assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))), "格式 {fmt} 应被拒");
        }
    }

    #[test]
    fn unknown_kind_is_rejected_instead_of_silently_converting() {
        let t = Tree::new("kind");
        let mut it = good_pair(&t);
        it.kind = "remux".into();
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))));
    }

    #[test]
    fn input_outside_the_authorized_scope_is_rejected() {
        let t = Tree::new("scope");
        let mut it = good_pair(&t);
        let other = Tree::new("scope-outside");
        it.path = other.file("base/a.flac");
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Outside(_))));
    }

    #[test]
    fn output_must_sit_in_the_source_directory() {
        let t = Tree::new("outdir");
        let mut it = good_pair(&t);
        it.out_path = t.path("elsewhere/a.mp3");
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))));
    }

    #[test]
    fn existing_output_is_never_overwritten() {
        let t = Tree::new("exists");
        let mut it = good_pair(&t);
        let existing = t.file("base/a.mp3");
        it.out_path = existing;
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))));
    }

    #[test]
    fn extension_must_match_the_mode() {
        let t = Tree::new("ext");
        let video = t.file("base/v.mkv");
        let mut it = item(&video, &t.path("base/v.mp3"), "convert", "mp3");
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))), "音频模式不吃视频");
        let audio = t.file("base/a.flac");
        it = item(&audio, &t.path("base/a.mp3"), "extract", "mp3");
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))), "提取模式不吃音频");
    }

    #[test]
    fn output_extension_must_match_the_format_and_never_be_the_source() {
        let t = Tree::new("outext");
        let mut it = good_pair(&t);
        it.out_path = t.path("base/a.flac");
        assert!(matches!(validate_items(&scope_for(&t), &[it]), Err(Reject::Invalid(_))), "扩展名不符要拒");

        // 与源同扩展名同目录 = 覆盖自己，同样拒
        let src = t.file("base/b.flac");
        let mut same = item(&src, &t.path("base/b.flac"), "convert", "flac");
        same.out_path = t.path("base/b.FLAC");
        assert!(matches!(validate_items(&scope_for(&t), &[same]), Err(Reject::Invalid(_))), "覆盖源文件要拒");
    }

    #[test]
    fn missing_or_scheme_shaped_inputs_are_rejected() {
        let t = Tree::new("missing");
        let scope = scope_for(&t);
        let mut it = good_pair(&t);
        it.path = t.path("base/not-there.flac");
        assert!(matches!(validate_items(&scope, &[it]), Err(Reject::Invalid(_))));

        let mut it = good_pair(&t);
        it.path = "http://example.com/a.mp3".into();
        assert!(matches!(validate_items(&scope, &[it]), Err(Reject::Invalid(_))));
        let mut it = good_pair(&t);
        it.path = "-i".into();
        assert!(matches!(validate_items(&scope, &[it]), Err(Reject::Invalid(_))));
    }

    #[test]
    fn an_empty_batch_is_rejected() {
        let t = Tree::new("emptybatch");
        assert!(matches!(validate_items(&scope_for(&t), &[]), Err(Reject::Invalid(_))));
    }

    #[test]
    fn a_valid_pair_passes_and_never_clobbers() {
        let t = Tree::new("ok");
        let scope = scope_for(&t);
        let audio = good_pair(&t);
        assert!(validate_items(&scope, std::slice::from_ref(&audio)).is_ok());

        let args = build_convert_args(&audio).expect("白名单格式应能构建参数");
        assert!(args.contains(&"-n".to_string()), "必须用 -n：目标已存在时失败而不是覆盖");
        assert!(!args.contains(&"-y".to_string()), "不允许 -y");
        assert!(!args.contains(&"copy".to_string()), "不再有音频流拷贝兜底");
        assert_eq!(args.last().unwrap(), &audio.out_path, "输出路径是最后一个参数");

        // extract：多一个 -vn（丢视频流），参数表之外没有任何自由位置
        let video = t.file("base/v.mp4");
        let ex = item(&video, &t.path("base/v.m4a"), "extract", "m4a");
        let ex_args = build_convert_args(&ex).expect("提取参数");
        assert!(ex_args.contains(&"-vn".to_string()));
        let i = ex_args.iter().position(|a| a == "-i").expect("有 -i");
        assert_eq!(ex_args[i + 1], ex.path, "输入紧跟在 -i 后面");
        assert_eq!(ex_args.last().unwrap(), &ex.out_path, "输出路径是最后一个参数");
        assert!(!ex_args.iter().any(|a| a == "-ac" || a == "-ar"), "保持源声道/采样率");
    }
}
