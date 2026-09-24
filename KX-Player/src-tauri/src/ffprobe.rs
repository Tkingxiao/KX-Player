//! ffprobe 探测（视频时长/码率/采样率）。
//!
//! 安全说明：等价 subprocess.run([...], shell=False) 的参数列表调用——
//! 固定 argv 数组，输入路径为最后一个独立参数且先做 is_file 校验；无 shell、无字符串拼接。
//!
//! P1-4：不再自己建运行时、用 `Command::output()` 整份收流（既无超时也无上限，卡住就不返回），
//! 改走 `fftools` 的统一子进程通道 —— 有超时、有输出上限、退出前会被 `kill_all` 收走。
//! ffprobe 的 JSON 在**开头**，所以 stdout 用「超限整份拒绝」（截断了也解析不了）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::fftools::{exec_capture, OutCap};
use crate::limits::{MAX_CHILD_TAIL_BYTES, MAX_FFPROBE_BYTES};

/// 单次探测的超时。正常文件秒级返回；卡住的（离线盘、坏文件、被占用的盘）到点就掐，
/// 不再让扫描线程无限等一个 ffprobe。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    sample_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeOut {
    format: Option<FfprobeFormat>,
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Clone, Default)]
pub struct ProbeMeta {
    pub duration: f64,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
}

/// 参数列表（无 shell、无拼接）：路径是最后一个独立参数，带空格/引号/`&` 也只是一个元素。
fn probe_argv(probe: &Path, path: &str) -> Vec<String> {
    vec![
        probe.to_string_lossy().into_owned(),
        "-v".into(),
        "error".into(),
        "-print_format".into(),
        "json".into(),
        "-show_format".into(),
        "-show_streams".into(),
        "-i".into(),
        path.into(),
    ]
}

/// 跑一次 ffprobe 收 stdout；读不到（起不来/超时/被杀/超限/非零退出）返回 `None`。
/// 只有「异常」那几类才记日志 —— 非媒体文件探不到是扫描的常态，不该刷日志。
fn run_probe(probe: &Path, path: &str) -> Option<Vec<u8>> {
    let argv = probe_argv(probe, path);
    let (exe, args) = argv.split_first()?;
    let c = exec_capture(
        PathBuf::from(exe),
        args,
        PROBE_TIMEOUT,
        OutCap::Head(MAX_FFPROBE_BYTES),
        OutCap::Tail(MAX_CHILD_TAIL_BYTES),
    );
    if let Some(e) = c.error {
        crate::paths::append_log(&format!("[ffprobe] 探测未完成：{e}（{path}）"));
        return None;
    }
    if c.stdout_capped {
        crate::paths::append_log(&format!("[ffprobe] 输出超过 {MAX_FFPROBE_BYTES} 字节，整份拒绝（{path}）"));
        return None;
    }
    if c.code != 0 {
        return None;
    }
    Some(c.stdout)
}

/// 用 ffprobe 读取媒体元数据。路径必须先通过 is_file 校验。
pub fn probe_meta(path: &str) -> Option<ProbeMeta> {
    if !std::path::Path::new(path).is_file() {
        return None;
    }
    let probe = crate::paths::locate_ffprobe()?;
    let stdout = run_probe(&probe, path)?;
    let parsed: FfprobeOut = serde_json::from_slice(&stdout).ok()?;
    let format = parsed.format?;
    Some(ProbeMeta {
        duration: format.duration.and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0),
        bitrate: format.bit_rate.and_then(|b| b.parse::<i64>().ok()),
        sample_rate: parsed
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("audio"))
            .and_then(|s| s.sample_rate.as_ref().and_then(|r| r.parse::<i64>().ok())),
    })
}

#[cfg(test)]
mod tests {
    use super::probe_argv;

    /// 参数列表式调用：**路径是最后一个独立参数**，且整个路径只占一个元素 ——
    /// 带空格、`&`、引号的路径不会被拆开或当成选项（无 shell 参与）。
    #[test]
    fn the_probe_path_stays_one_single_argument() {
        let p = r"C:\Music\a & b; 'c'.mp3";
        let argv = probe_argv(std::path::Path::new(r"C:\tools\ffprobe.exe"), p);
        assert_eq!(argv[0], r"C:\tools\ffprobe.exe");
        assert_eq!(argv.last().unwrap(), p);
        assert_eq!(argv[argv.len() - 2], "-i", "路径紧跟 -i，表外没有自由位置");
        assert!(argv.contains(&"json".to_string()));
        assert!(argv.contains(&"-show_format".to_string()));
        assert!(argv.contains(&"-show_streams".to_string()));
        assert_eq!(argv.iter().filter(|a| *a == p).count(), 1, "路径只出现一次");
    }
}
