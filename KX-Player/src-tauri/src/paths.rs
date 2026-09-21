//! 数据目录解析。
//! 关键决策：继续沿用 Electron 版的 userData 目录（%APPDATA%/kx-music-player/Cache/kx-music-player），
//! 老用户的曲库 SQLite / 设置 / 封面 / 背景图 / 进度 / 书签全部原地继承，迁移零成本。

use std::path::PathBuf;

fn appdata() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
}

/// 唯一数据目录（与 Electron 版完全一致）
pub fn data_dir() -> PathBuf {
    appdata().join("kx-music-player").join("Cache").join("kx-music-player")
}

pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn settings_path() -> PathBuf {
    data_dir().join("kx-player-settings.json")
}

pub fn library_db_path() -> PathBuf {
    data_dir().join("kx-player-library.sqlite")
}

pub fn covers_dir() -> PathBuf {
    data_dir().join("covers")
}

pub fn ensure_covers_dir() -> std::io::Result<PathBuf> {
    let dir = covers_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn bg_image_path() -> PathBuf {
    data_dir().join("kx-player-bg.png")
}

pub fn log_path() -> PathBuf {
    data_dir().join("kx-player-log.txt")
}

/// 定位 ffmpeg.exe：环境变量 → exe 旁 / 资源目录 → 常见安装位置 → PATH
pub fn locate_ffmpeg() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KX_FFMPEG") {
        if std::path::Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
            // NSIS resources 目录
            let candidate = dir.join("ffmpeg").join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    for p in ["C:\\ffmpeg\\bin\\ffmpeg.exe", "C:\\Program Files\\ffmpeg\\bin\\ffmpeg.exe"] {
        let candidate = PathBuf::from(p);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn locate_ffprobe() -> Option<PathBuf> {
    locate_ffmpeg().map(|ff| {
        let probe = ff.with_file_name("ffprobe.exe");
        if probe.exists() {
            probe
        } else {
            ff.with_file_name("ffprobe.exe")
        }
    })
}

/// 追加一行日志（保留最近 1MB，与 Electron 版行为一致）
pub fn append_log(msg: &str) {
    use std::io::Write;
    let path = log_path();
    let ts = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let (y, mo, d, h, mi, s) = epoch_to_utc(secs);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    };
    let line = format!("[{ts}] {msg}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
    // 超过 1MB 时**原地保留尾部 512KB**（不是多文件滚动，注释别写成人话之外的意思）
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1024 * 1024 {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let keep_from = tail_start(&content, 512 * 1024);
                let _ = std::fs::write(&path, &content[keep_from..]);
            }
        }
    }
}

/// 取「保留尾部 `keep` 字节」的起点，且**必须落在字符边界与行首**。
///
/// 直接 `&content[content.len() - keep..]` 是字节切片：日志里有中文，切点落在多字节
/// 字符中间就 panic（本仓在 `ai.rs` 踩过同一类坑，`brief` 是那次留下的）。
/// 这里再顺带对齐到行首，免得日志开头是半行。
fn tail_start(content: &str, keep: usize) -> usize {
    let mut start = content.len().saturating_sub(keep);
    while start < content.len() && !content.is_char_boundary(start) {
        start += 1;
    }
    match content[start..].find('\n') {
        Some(i) => start + i + 1,
        None => start,
    }
}


/// 简单 UTC 时间分解（避免引入 chrono）
fn epoch_to_utc(epoch: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = epoch / 86400;
    let rem = epoch % 86400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil_from_days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u64, m as u64, d as u64, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::tail_start;

    /// 老写法 `&content[content.len() - keep..]` 在中文日志上会 panic
    #[test]
    fn tail_start_never_splits_a_character() {
        let mut log = String::new();
        for i in 0..200 {
            log.push_str(&format!("[{i:04}] 播放失败：设备被占用\n"));
        }
        for keep in 1..4096 {
            let start = tail_start(&log, keep);
            assert!(log.is_char_boundary(start), "keep={keep} 切在字符中间");
            assert!(log[start..].starts_with('[') || start == log.len(), "keep={keep} 没对齐行首");
        }
    }

    #[test]
    fn tail_start_keeps_everything_when_short() {
        let s = "一行";
        assert_eq!(tail_start(s, 4096), 0);
    }
}
