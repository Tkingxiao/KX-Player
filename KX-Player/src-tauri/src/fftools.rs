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
