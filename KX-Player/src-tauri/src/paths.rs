//! 数据目录解析。
//! 关键决策：用户数据一律落在**安装目录**旁的 `data/`（WebView 的用户数据文件夹也在里面），
//! 不写 `%APPDATA%` / `%LOCALAPPDATA%`。跟着安装目录走：装两份互不串数据，拷走即搬家。
//! 代价是安装目录必须可写 —— 装进 `Program Files` 这类受保护位置时数据存不下来，
//! 这种情况如实警告（见 `lib.rs` 的启动自检），而不是假装一切正常。
//! 老位置 `%APPDATA%/kx-music-player/Cache/kx-music-player` 里的东西在首次启动时整体搬过来
//! （[`migrate_legacy_data`]），升上来的用户曲库不空。

use std::path::{Path, PathBuf};

/// 安装目录 = exe 所在目录。NSIS 把 exe 放在 `$INSTDIR` 根，`libmpv-2.dll` 与它同级。
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 唯一数据目录：`<安装目录>/data`。
pub fn data_dir() -> PathBuf {
    install_dir().join("data")
}

/// WebView 的用户数据文件夹（localStorage / Cookie / 缓存都归它，主窗口与悬浮窗共用一份）。
/// 不显式指定的话 Tauri 会把它塞进 `%LOCALAPPDATA%/<identifier>`。
pub fn webview_data_dir() -> PathBuf {
    data_dir().join("webview")
}

/// 数据目录可用时才交出 WebView 的自定义位置：安装目录不可写时让 Tauri 回落到它自己的
/// 默认位置 —— 那个目录建不出来时窗口根本起不来，比数据落错地方更糟。
pub fn webview_data_dir_if_usable() -> Option<PathBuf> {
    let dir = webview_data_dir();
    std::fs::create_dir_all(&dir).ok().map(|_| dir)
}

/// 老版本（沿用 Electron 时代的 userData 布局）的数据目录；不存在或环境里没有 `%APPDATA%` 时给 `None`。
fn legacy_data_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok().filter(|s| !s.is_empty())?;
    let dir = PathBuf::from(appdata)
        .join("kx-music-player")
        .join("Cache")
        .join("kx-music-player");
    dir.is_dir().then_some(dir)
}

/// 把老位置的数据一次性搬进 `<安装目录>/data`。返回搬过来的条目数（0 = 没搬）。
///
/// 判据只有两条：老目录里有东西、新目录还是空的。新目录非空说明这台机器已经在用新位置，
/// 那时候动它就不是迁移而是覆盖了。搬不动的条目留在老位置原样不动，下次启动再试。
pub fn migrate_legacy_data() -> usize {
    match legacy_data_dir() {
        Some(old) => migrate_from(&old, &data_dir()),
        None => 0,
    }
}

/// 迁移的主体（`migrate_legacy_data` 只负责把两个目录递进来）：拆开是为了能在临时目录上测。
fn migrate_from(old: &Path, new: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(old) else { return 0 };
    let names: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    if names.is_empty() || !is_empty_dir(new) {
        return 0;
    }
    let mut moved = 0;
    for name in names {
        let (src, dst) = (old.join(&name), new.join(&name));
        match move_into(&src, &dst) {
            Ok(()) => moved += 1,
            Err(e) => append_log(&format!(
                "[迁移] {} 搬不进安装目录：{e}（留在老位置，下次启动再试）",
                src.display()
            )),
        }
    }
    moved
}

/// 目录不存在与目录为空在这里等价（都是「可以往里搬」）；读不动目录时按非空处理，宁可不动。
fn is_empty_dir(p: &Path) -> bool {
    match std::fs::read_dir(p) {
        Ok(mut d) => d.next().is_none(),
        Err(_) => true,
    }
}

/// 把 `src` 整个挪到 `dst`：先 `rename`（同盘一步到位），失败再「复制 + 删源」——
/// 老位置在 C 盘、安装目录在别的盘时走的就是后一条。
fn move_into(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    if let Err(e) = copy_tree(src, dst) {
        // 半份复制比不搬更糟：清掉半成品，老数据保持完整，下次启动还会再试一遍。
        let _ = if dst.is_dir() {
            std::fs::remove_dir_all(dst)
        } else {
            std::fs::remove_file(dst)
        };
        return Err(e);
    }
    if src.is_dir() {
        std::fs::remove_dir_all(src)
    } else {
        std::fs::remove_file(src)
    }
}

/// 递归复制，目标的父目录不存在时一并建出来。
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.is_dir() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        return std::fs::copy(src, dst).map(|_| ());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(())
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

/// ffmpeg.exe 是从哪儿找到的（P1-12：来路要能说清楚）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FfmpegSource {
    /// 环境变量 `KX_FFMPEG`
    Env,
    /// 程序目录（含 NSIS resources 子目录）
    AppDir,
    /// 常见安装位置（写死的两个路径）
    Fixed,
    /// PATH 里第一个同名文件
    Path,
}

impl FfmpegSource {
    pub fn label(self) -> &'static str {
        match self {
            FfmpegSource::Env => "环境变量 KX_FFMPEG",
            FfmpegSource::AppDir => "程序目录",
            FfmpegSource::Fixed => "常见安装位置",
            FfmpegSource::Path => "PATH",
        }
    }
}

/// 定位 ffmpeg.exe：环境变量 → exe 旁 / 资源目录 → 常见安装位置 → PATH。
/// 返回来路，供启动日志与转换页提示（[`FfmpegSource`]）。
pub fn locate_ffmpeg_ex() -> Option<(PathBuf, FfmpegSource)> {
    if let Ok(p) = std::env::var("KX_FFMPEG") {
        if std::path::Path::new(&p).exists() {
            return Some((PathBuf::from(p), FfmpegSource::Env));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("ffmpeg.exe");
            if candidate.exists() {
                return Some((candidate, FfmpegSource::AppDir));
            }
            // NSIS resources 目录
            let candidate = dir.join("ffmpeg").join("ffmpeg.exe");
            if candidate.exists() {
                return Some((candidate, FfmpegSource::AppDir));
            }
        }
    }
    for p in ["C:\\ffmpeg\\bin\\ffmpeg.exe", "C:\\Program Files\\ffmpeg\\bin\\ffmpeg.exe"] {
        let candidate = PathBuf::from(p);
        if candidate.exists() {
            return Some((candidate, FfmpegSource::Fixed));
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("ffmpeg.exe");
            if candidate.exists() {
                return Some((candidate, FfmpegSource::Path));
            }
        }
    }
    None
}

pub fn locate_ffmpeg() -> Option<PathBuf> {
    locate_ffmpeg_ex().map(|(p, _)| p)
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
                let _ = crate::atomic::write(&path, &content.as_bytes()[keep_from..]);
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
    use super::{is_empty_dir, migrate_from, move_into, tail_start};
    use std::path::{Path, PathBuf};

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

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kx-migrate-{tag}-{}-{nanos}", std::process::id()))
    }

    fn touch(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// 老目录里有曲库和封面、新目录还没用过：整份搬过去，老目录剩下空壳。
    #[test]
    fn migrate_moves_the_old_layout_into_the_new_dir() {
        let root = temp_root("move");
        let (old, new) = (root.join("old"), root.join("new"));
        touch(&old.join("kx-player-library.sqlite"), "库");
        touch(&old.join("covers").join("a.jpg"), "封面");
        std::fs::create_dir_all(&new).unwrap();

        assert_eq!(migrate_from(&old, &new), 2);
        assert_eq!(std::fs::read_to_string(new.join("kx-player-library.sqlite")).unwrap(), "库");
        assert_eq!(std::fs::read_to_string(new.join("covers").join("a.jpg")).unwrap(), "封面");
        assert!(is_empty_dir(&old), "老目录该空了");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 新目录非空 = 这台机器已经在用新位置，那时再搬就是覆盖，一条都不许动。
    #[test]
    fn migrate_never_touches_a_new_dir_already_in_use() {
        let root = temp_root("busy");
        let (old, new) = (root.join("old"), root.join("new"));
        touch(&old.join("kx-player-settings.json"), "老设置");
        touch(&new.join("kx-player-settings.json"), "新设置");

        assert_eq!(migrate_from(&old, &new), 0);
        assert_eq!(std::fs::read_to_string(new.join("kx-player-settings.json")).unwrap(), "新设置");
        assert!(old.join("kx-player-settings.json").is_file(), "老数据该原样留着");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_is_a_noop_when_there_is_nothing_to_move() {
        let root = temp_root("noop");
        let new = root.join("new");
        // 老目录不存在
        assert_eq!(migrate_from(&root.join("missing"), &new), 0);
        // 老目录在、但里面没东西
        std::fs::create_dir_all(root.join("old")).unwrap();
        assert_eq!(migrate_from(&root.join("old"), &new), 0);
        assert!(!new.exists(), "没得搬时不该顺手建出新目录");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `rename` 失败（这里用「目标是个非空目录」逼出来，跨盘时也是同一条路）时，
    /// 退化成复制后删源：内容要合得进去，源不能留半份。
    #[test]
    fn move_into_falls_back_to_copy_when_rename_cannot_work() {
        let root = temp_root("fallback");
        let (src, dst) = (root.join("src"), root.join("dst"));
        touch(&src.join("covers").join("a.jpg"), "封面");
        touch(&dst.join("covers").join("b.jpg"), "已有的");

        move_into(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("covers").join("a.jpg")).unwrap(), "封面");
        assert_eq!(std::fs::read_to_string(dst.join("covers").join("b.jpg")).unwrap(), "已有的");
        assert!(!src.exists(), "搬完了老位置不该还剩东西");

        let _ = std::fs::remove_dir_all(&root);
    }
}
