//! 响度测量：ffmpeg 只负责解码，EBU R128 的积分响度由 Rust 侧算。
//!
//! 旧实现是 `ffmpeg -af ebur128 -f null NUL` 再从 stderr 里正则捞
//! `I:  -23.5 LUFS`。那条路把「测量结果」压在一行**文案**上：ffmpeg 改措辞、
//! 换 locale、多打一行进度，都会让它静默变成「分析失败」，而且 Peak/LRA 想要
//! 哪个就得再解析一段。这里改成让 ffmpeg 把 PCM 吐到 stdout（`-f f32le`），
//! 边读边喂 `ebur128`（纯 Rust 实现，只吃 PCM、不解码）：
//! 全程不解析文本，内存与文件时长无关（读完就丢），要更多指标时分析器已经在手边。
//!
//! 解码仍然要 ffmpeg —— 播放器引擎是 mpv，本仓不分发第二个解码器栈。

use std::path::Path;
use std::time::Duration;

use ebur128::{EbuR128, Mode};

/// 分析用采样率。`ebur128` 会按采样率重算 K 加权系数（任意采样率都支持），
/// 但分析器必须在**开工前**建好，而源文件的采样率要再探一次才知道 ——
/// 于是统一让 ffmpeg 重采样到 48 kHz：一次 swresample 换掉一次 ffprobe 子进程。
pub const SAMPLE_RATE: u32 = 48_000;
/// 声道数固定为 2（配合 `-ac 2`）。R128 的多声道加权（环绕按 C 计）这里不做，
/// 5.1/7.1 素材走 ffmpeg 的 downmix —— 与播放器实际会做的事一致。
pub const CHANNELS: u32 = 2;
/// 单文件上限。和滤镜法同量级（都要解整轨），慢盘 + 长音频留足余量。
const TIMEOUT: Duration = Duration::from_secs(300);
/// 一次读多少字节。必须是**一帧的整数倍**（见 [`FRAME_BYTES`]），否则凑帧的
/// 余数会每块都出现一次。
const READ_CHUNK_BYTES: usize = 64 * 1024;
/// 一帧的字节数：双声道 × f32。
const FRAME_BYTES: usize = (CHANNELS as usize) * 4;
/// 低于此值按「测不出有效响度」处理。数字静音的读数是 `-inf`，
/// 16 bit 的噪声底也在 -96 dB 以下，所以这条线不会误伤真实素材。
const MIN_VALID_LUFS: f64 = -100.0;

/// 测量失败的原因。`Ok(None)` 与 `Err` 是两件事：静音是**测准了**（没有响度），
/// 失败是**没测成**，UI 上不该算进同一个计数。
#[derive(Debug, thiserror::Error)]
pub enum LoudnessError {
    #[error("ffmpeg.exe 未找到")]
    FfmpegMissing,
    #[error("{0}")]
    Unavailable(String),
    #[error("ffmpeg 退出码 {code}：{tail}")]
    Failed { code: i32, tail: String },
    #[error("响度分析器不可用: {0}")]
    Analyzer(String),
}

/// 测一个文件的积分响度（LUFS）。
///
/// * `Ok(Some(v))` —— 正常结果
/// * `Ok(None)` —— 数字静音／测不出有效值（不是失败）
/// * `Err(_)` —— 起不来、超时、退出中、ffmpeg 报错
pub fn measure_loudness(path: &Path) -> Result<Option<f64>, LoudnessError> {
    let Some(exe) = crate::paths::locate_ffmpeg() else {
        return Err(LoudnessError::FfmpegMissing);
    };
    let mut analyzer = EbuR128::new(CHANNELS, SAMPLE_RATE, Mode::I)
        .map_err(|e| LoudnessError::Analyzer(e.to_string()))?;
    let mut feeder = Feeder::new(&mut analyzer);
    let args = measure_args(path);

    let streamed = crate::fftools::exec_stream(exe, &args, TIMEOUT, READ_CHUNK_BYTES, |chunk| {
        feeder.push(chunk)
    });
    // 超时 / 退出途中被收走 / 起不来：`error` 已经写好了人话，直接透传
    if let Some(e) = streamed.error {
        return Err(LoudnessError::Unavailable(e));
    }
    if let Err(e) = feeder.finish() {
        return Err(LoudnessError::Analyzer(e));
    }
    if streamed.code != 0 {
        return Err(LoudnessError::Failed { code: streamed.code, tail: streamed.stderr_tail });
    }
    let v = analyzer
        .loudness_global()
        .map_err(|e| LoudnessError::Analyzer(e.to_string()))?;
    Ok(usable_lufs(v))
}

/// `-i` 的参数字面量。路径始终是**独立的一个参数元素**（无 shell 参与），
/// 和转换链路用的是同一条安全约束。
fn measure_args(path: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-nostats".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        path.to_string_lossy().into_owned(),
        // 只取第一条音轨：视频里的第二音轨不该改变整条曲目的响度结论，
        // 而封面图会被当成视频流解码 —— `-vn -sn` 把它和字幕一起关掉。
        "-vn".into(),
        "-sn".into(),
        "-map".into(),
        "0:a:0".into(),
        "-ac".into(),
        CHANNELS.to_string(),
        "-ar".into(),
        SAMPLE_RATE.to_string(),
        "-f".into(),
        "f32le".into(),
        "-acodec".into(),
        "pcm_f32le".into(),
        "pipe:1".into(),
    ]
}

/// 读数是否是一个可用的响度值。`-inf`（数字静音）与 NaN 都不是。
fn usable_lufs(v: f64) -> Option<f64> {
    (v.is_finite() && v > MIN_VALID_LUFS).then_some(v)
}

/// 字节流 → 整帧 f32 → 分析器。
///
/// 管道随时可能把一个样本（4 字节）甚至一帧（8 字节）劈在两块之间，而
/// `add_frames_f32` 要求样本总数是声道数的整数倍（凑不满就返回 Err），
/// 所以这里留最多 7 字节的尾巴接到下一块前面。`finish` 时尾巴必须为空 ——
/// 非空说明流被截断，那半帧不能当响度算。
struct Feeder<'a> {
    r128: &'a mut EbuR128,
    pending: Vec<u8>,
    samples: Vec<f32>,
}

impl<'a> Feeder<'a> {
    fn new(r128: &'a mut EbuR128) -> Self {
        Self { r128, pending: Vec::with_capacity(FRAME_BYTES * 2), samples: Vec::new() }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.pending.extend_from_slice(bytes);
        // 凑不满一帧的尾巴（最多 7 字节）留到下一块：管道能把一个样本劈成两半
        let usable = self.pending.len() / FRAME_BYTES * FRAME_BYTES;
        if usable == 0 {
            return Ok(());
        }
        // f32le：小端四字节一个样本。分析器要的是交错布局，正好是管道里的顺序。
        self.samples.clear();
        self.samples.reserve(usable / 4);
        let mut at = 0;
        while at < usable {
            let w = &self.pending[..usable][at..at + FRAME_BYTES];
            self.samples.push(f32::from_le_bytes([w[0], w[1], w[2], w[3]]));
            self.samples.push(f32::from_le_bytes([w[4], w[5], w[6], w[7]]));
            at += FRAME_BYTES;
        }
        self.pending.drain(..usable);
        self.r128.add_frames_f32(&self.samples).map_err(|e| format!("ebur128: {e}"))
    }

    /// 收尾：确认没有半帧悬着。
    fn finish(&self) -> Result<(), String> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(format!("PCM 流在帧中间结束（剩 {} 字节）", self.pending.len()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一段双声道交错 f32 的字节流（小端，和 ffmpeg 输出口径一致）。
    fn pcm(interleaved: &[f32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(interleaved.len() * 4);
        for s in interleaved {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    /// 一个 `frames` 帧的正弦，交错双声道。
    fn sine(frames: usize, amp: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = amp * ((i as f64 * 0.2).sin() as f32);
            v.push(s);
            v.push(s);
        }
        v
    }

    fn analyzer() -> EbuR128 {
        EbuR128::new(CHANNELS, SAMPLE_RATE, Mode::I).expect("2ch/48k 是合法参数")
    }

    #[test]
    fn args_keep_the_path_as_one_element_and_ask_for_raw_pcm() {
        let a = measure_args(Path::new(r"C:\x y\带 空格 与'引号'.flac"));
        let i = a.iter().position(|x| x == "-i").expect("-i 在参数里");
        assert_eq!(a[i + 1], r"C:\x y\带 空格 与'引号'.flac", "路径必须独占一个参数元素");
        assert_eq!(a.last().map(String::as_str), Some("pipe:1"));
        assert!(a.windows(2).any(|w| w[0] == "-f" && w[1] == "f32le"));
        assert!(a.windows(2).any(|w| w[0] == "-ar" && w[1] == SAMPLE_RATE.to_string()));
        assert!(a.windows(2).any(|w| w[0] == "-ac" && w[1] == CHANNELS.to_string()));
    }

    #[test]
    fn silence_measures_no_valid_loudness() {
        let mut a = analyzer();
        let mut f = Feeder::new(&mut a);
        // 48k 双声道 2 秒：R128 的块是 400 ms（滑动 100 ms），要够长才有块
        let chunk = pcm(&vec![0.0f32; (CHANNELS as usize) * 1000]);
        for _ in 0..100 {
            f.push(&chunk).expect("喂零");
        }
        f.finish().expect("整帧收尾");
        assert_eq!(usable_lufs(a.loudness_global().unwrap()), None, "数字静音不该给出响度值");
    }

    #[test]
    fn a_tone_measures_a_loudness_and_doubling_it_adds_six_db() {
        let mut quiet = analyzer();
        let mut loud = analyzer();
        let tone = (CHANNELS as usize) * SAMPLE_RATE as usize * 3; // 3 秒
        {
            let mut f = Feeder::new(&mut quiet);
            f.push(&pcm(&sine(tone / 2, 0.2))).expect("喂入");
            f.finish().expect("整帧收尾");
        }
        {
            let mut f = Feeder::new(&mut loud);
            f.push(&pcm(&sine(tone / 2, 0.4))).expect("喂入");
            f.finish().expect("整帧收尾");
        }
        let q = usable_lufs(quiet.loudness_global().unwrap()).expect("正弦该有响度");
        let l = usable_lufs(loud.loudness_global().unwrap()).expect("正弦该有响度");
        assert!(q > MIN_VALID_LUFS && q < 0.0, "幅值 0.2 的正弦读数 {q} 不在合理区间");
        assert!((l - q - 6.0).abs() < 0.2, "幅值翻倍应当 +6 dB，实测 +{}", l - q);
    }

    #[test]
    fn frames_survive_being_split_at_every_byte_offset() {
        // 同一条 PCM 流，按每个可能的错位切开喂进去，读数必须与整块喂一致
        let bytes = pcm(&sine(CHANNELS as usize * 2500, 0.3));
        let mut whole = analyzer();
        Feeder::new(&mut whole).push(&bytes).expect("整块喂");

        for split in 1..FRAME_BYTES * 2 {
            let mut a = analyzer();
            let mut f = Feeder::new(&mut a);
            for chunk in bytes.chunks(split.max(1)) {
                f.push(chunk).unwrap_or_else(|e| panic!("错位 {split} 处喂入失败：{e}"));
            }
            f.finish().unwrap_or_else(|e| panic!("错位 {split} 收尾失败：{e}"));
            assert_eq!(
                a.loudness_global().unwrap().to_bits(),
                whole.loudness_global().unwrap().to_bits(),
                "切分位置不该改变读数（凑不满一帧的字节要留到下一块）"
            );
        }
    }

    #[test]
    fn a_ragged_tail_is_reported_instead_of_being_dropped() {
        let mut a = analyzer();
        let mut f = Feeder::new(&mut a);
        let mut bytes = pcm(&sine(CHANNELS as usize * 1000 / 2, 0.3));
        bytes.truncate(bytes.len() - 3); // 砍掉半个样本
        f.push(&bytes).expect("喂入");
        assert!(f.finish().is_err(), "剩 5 字节说明流被截断，不能当它不存在");
    }

    #[test]
    fn only_finite_values_above_the_floor_are_usable() {
        assert_eq!(usable_lufs(-23.5), Some(-23.5));
        assert_eq!(usable_lufs(f64::NEG_INFINITY), None, "-inf = 数字静音");
        assert_eq!(usable_lufs(f64::NAN), None, "NaN 不是响度");
        assert_eq!(usable_lufs(-140.0), None, "低于噪声底按无有效响度");
    }
}
