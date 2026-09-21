//! 元数据解析：音频 lofty（快），视频/未知格式走 ffprobe（见 ffprobe.rs，参数列表式调用）。

use crate::model::normalize_path;
use lofty::prelude::*;
use lofty::probe::Probe;
use serde::Serialize;

pub const AUDIO_EXTS: &[&str] = &["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "ape", "wv", "aiff", "alac"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv"];

pub fn ext_of(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_lowercase()
}

pub fn is_video(path: &str) -> bool {
    VIDEO_EXTS.contains(&ext_of(path).as_str())
}

/// 单文件解析结果。cover 指向已落盘的 covers/{trackId}.jpg（解析期内嵌封面直接保存，
/// 避免把所有 base64 常驻内存——老 Electron 实现的主要内存压力点）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanMeta {
    pub duration: f64,
    pub cover: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
}

/// 乱码检测：Latin-1 补充区占比 >20% 视为 Shift-JIS 被误读（与 Electron metadata-worker 一致）
fn is_likely_garbled(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut garbled = 0usize;
    for ch in text.chars() {
        let code = ch as u32;
        if (0x80..=0x9F).contains(&code) || (0xA1..=0xFF).contains(&code) {
            garbled += 1;
        }
    }
    garbled as f32 / text.chars().count() as f32 > 0.2
}

fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x3040..=0x30FF).contains(&u) || (0xFF00..=0xFFEF).contains(&u) || (0x4E00..=0x9FFF).contains(&u)
    })
}

pub fn try_fix_encoding(text: &str) -> String {
    if text.is_empty() || !is_likely_garbled(text) {
        return text.to_string();
    }
    let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
    for enc in [encoding_rs::SHIFT_JIS, encoding_rs::SHIFT_JIS] {
        let (decoded, _, had_errors) = enc.decode(&bytes);
        if !had_errors && contains_cjk(&decoded) {
            return decoded.to_string();
        }
    }
    text.to_string()
}

/// 条目 ID 命名空间（02 §1.1 UUIDv5）。固定常量，**换掉等于给全库换 ID**：
/// 进度、书签、分类、标签、封面文件与 settings 里的 trackId 全部按旧命名空间寻址。
const ID_NAMESPACE: uuid::Uuid = uuid::Uuid::from_u128(0xb9ab_1b7b_af50_4eab_98a9_ce5a_0d76_8fa9);

pub fn hash_path_id(path: &str) -> String {
    uuid::Uuid::new_v5(&ID_NAMESPACE, path.as_bytes()).simple().to_string()
}

fn lofty_supports(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "aiff" | "aif" | "ape" | "wv" | "alac"
    )
}

/// 解析音频文件（lofty）。失败返回 None（由调用方兜底）。
fn parse_audio_lofty(path: &str) -> Option<ScanMeta> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let props = tagged.properties();
    let duration = props.duration().as_secs_f64();
    let bitrate_kbps = props.audio_bitrate().unwrap_or(0);
    let sample_rate = props.sample_rate().unwrap_or(0);

    let mut meta = ScanMeta {
        duration,
        bitrate: if bitrate_kbps > 0 { Some(bitrate_kbps as i64 * 1000) } else { None },
        sample_rate: if sample_rate > 0 { Some(sample_rate as i64) } else { None },
        ..Default::default()
    };

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        meta.title = tag.title().map(|s| try_fix_encoding(s.trim()));
        meta.artist = tag.artist().map(|s| try_fix_encoding(s.trim()));
        meta.genre = tag.genre().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        // 内嵌封面直接压缩落盘（≤15MB）
        if let Some(pic) = tag.pictures().first() {
            let data = pic.data();
            if !data.is_empty() && data.len() <= 15 * 1024 * 1024 {
                let track_id = file_id(path);
                if let Some(saved) = crate::scanner::covers::save_cover_bytes(&track_id, data) {
                    meta.cover = Some(saved);
                }
            }
        }
    }
    Some(meta)
}

/// 解析单个媒体文件；失败时用文件名兜底（与 worker 的 extractBasicInfo 一致）
pub fn parse_file(path: &str) -> ScanMeta {
    let ext = ext_of(path);
    let mut meta: Option<ScanMeta> = None;
    if is_video(path) {
        if let Some(p) = crate::ffprobe::probe_meta(path) {
            meta = Some(ScanMeta { duration: p.duration, bitrate: p.bitrate, sample_rate: p.sample_rate, ..Default::default() });
        }
    } else if lofty_supports(&ext) {
        meta = parse_audio_lofty(path);
        // lofty 拿不到时长时（异常文件）用 ffprobe 补
        if meta.as_ref().is_some_and(|m| m.duration <= 0.0) {
            if let Some(p) = crate::ffprobe::probe_meta(path) {
                if p.duration > 0.0 {
                    let cover = meta.and_then(|m| m.cover);
                    meta = Some(ScanMeta { cover, duration: p.duration, bitrate: p.bitrate, sample_rate: p.sample_rate, ..Default::default() });
                }
            }
        }
    } else if let Some(p) = crate::ffprobe::probe_meta(path) {
        meta = Some(ScanMeta { duration: p.duration, bitrate: p.bitrate, sample_rate: p.sample_rate, ..Default::default() });
    }
    meta.unwrap_or(ScanMeta {
        duration: 0.0,
        cover: None,
        title: Some(basename_no_ext(path)),
        artist: None,
        genre: None,
        bitrate: None,
        sample_rate: None,
    })
}

pub fn basename_no_ext(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// 规范化显示名（去序号前缀、_/- 转空格），与 normalizeName 一致
pub fn normalize_display_name(path: &str) -> String {
    let raw = basename_no_ext(path);
    let trimmed = raw.trim_start_matches(|c: char| c.is_ascii_digit());
    let trimmed = if trimmed.len() < raw.len() {
        trimmed.trim_start_matches([' ', '.', '-', '_'])
    } else {
        trimmed
    };
    let out = trimmed.replace(['_', '-'], " ").trim().to_string();
    if out.is_empty() { raw } else { out }
}

pub fn stat_file(path: &str) -> Option<(f64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)?;
    Some((mtime, meta.len() as i64))
}

/// 条目稳定 ID：UUIDv5(规范化反斜杠路径)，32 位十六进制。
/// 路径规范化必须与 Electron 版 `hashPath` 的**输入**逐字符一致（Node `path.join` 在 Windows
/// 产生纯反斜杠路径）——ID 只随规范化规则变，不随哈希算法变；换算法（md5[..12] → UUIDv5）
/// 时由 `db::migrate_track_ids` 在启动期把旧 ID 全量改写，否则封面缓存、进度、书签、
/// 分类与 settings 里的 trackId 会全部失联（2026-09-19 已因此丢过一次进度）。
pub fn file_id(path: &str) -> String {
    let win_path = normalize_path(path).replace('/', "\\");
    hash_path_id(&win_path)
}

/// 读文件头探测图片尺寸（封面打分用，不解码全图）
pub fn image_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    let reader = image::ImageReader::open(path).ok()?;
    reader.into_dimensions().ok()
}

#[cfg(test)]
mod tests {
    use super::file_id;

    /// 钉住「命名空间 + 路径规范化」两件事：换掉任一项都会给全库换 ID，
    /// 进度、书签、收藏与封面缓存集体失联（见 db::migrate_legacy_ids）。
    #[test]
    fn file_id_golden_pins_namespace() {
        assert_eq!(file_id(r"C:\Music\Album\song.mp3"), "3a8fa096e5015b8090a414676dc28ebd");
    }

    /// ID 只由规范化后的路径决定：分隔符写法与结尾斜杠不该改变它。
    #[test]
    fn file_id_ignores_separators_and_trailing_slash() {
        let back = file_id(r"C:\Music\song.mp3");
        assert_eq!(back, file_id("C:/Music/song.mp3"));
        assert_eq!(back, file_id(r"C:\Music\song.mp3\"));
        assert_ne!(back, file_id(r"C:\Music\song.flac"));
    }
}
