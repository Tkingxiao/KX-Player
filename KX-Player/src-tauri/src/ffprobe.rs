//! ffprobe 探测（视频时长/码率/采样率）。
//! 安全说明：等价 subprocess.run([...], shell=False) 的参数列表调用——
//! 固定 argv 数组，输入路径为最后一个独立参数且先做 is_file 校验；无 shell、无字符串拼接。

use serde::Deserialize;

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

fn run_probe(argv: Vec<String>) -> Option<Vec<u8>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let out = rt.block_on(async {
        match tokio::process::Command::new(argv[0].clone()).args(&argv[1..]).output().await {
            Ok(o) => Some(o),
            Err(_) => None,
        }
    })?;
    if out.status.success() {
        Some(out.stdout)
    } else {
        None
    }
}

/// 用 ffprobe 读取媒体元数据。路径必须先通过 is_file 校验。
pub fn probe_meta(path: &str) -> Option<ProbeMeta> {
    if !std::path::Path::new(path).is_file() {
        return None;
    }
    let probe = crate::paths::locate_ffprobe()?;
    let argv: Vec<String> = vec![
        probe.to_string_lossy().into_owned(),
        "-v".into(), "error".into(),
        "-print_format".into(), "json".into(),
        "-show_format".into(), "-show_streams".into(),
        "-i".into(), path.into(),
    ];
    let stdout = run_probe(argv)?;
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
