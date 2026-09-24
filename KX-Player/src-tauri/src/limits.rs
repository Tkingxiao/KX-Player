//! 读取限额（P0-1 后半）。
//!
//! 这些入口读的是「用户自己选的文件」—— 不是攻击面，但一次误选（把 4K 原盘视频当封面、
//! 把几十 GB 的日志当歌词）或一条坏数据（解码炸弹：几十 KB 的 PNG 解出上亿像素）就能把
//! WebView 或进程内存顶穿。限额的意义是把「读不动」变成一条**说得出原因的拒绝**，
//! 而不是让进程 OOM 之后什么都不剩。
//!
//! 数值取「真的用得上的最大值再留一倍」：正常的歌词/字幕/译文在几百 KB 以内，
//! 封面原图最大见过 15 MB 级，背景图上传是前端 canvas 剪裁后的结果。

use std::path::Path;

/// 单次读文本的上限（歌词 `.lrc`、字幕、AI-16 译文导入走的都是 `read_text_file`）。
pub const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;
/// 单次读图片原始字节的上限（封面、背景图选择）。
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// 单张图片解码后的像素上限。**这一条防的是解码炸弹，不是大图**：PNG 的 DEFLATE 压缩比
/// 可以到千倍以上，1 MB 的输入能解出 10 亿像素（≈4 GB RGBA）。解码前先量尺寸。
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
/// 目录列举的条数上限（`list_dir` 只服务歌词同名文件查找与目录浏览，不做全库索引）。
pub const MAX_DIR_ENTRIES: usize = 20_000;
/// 单次 HTTP 响应体的上限（AI 对话、模型清单）。对流式/无 `Content-Length` 的响应
/// 也要在读到上限时停下，不能等对方把话说完。
pub const MAX_AI_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// 子进程（ffmpeg）输出的**尾部**窗口：有价值的信息（报错、ebur128 汇总）都在最后几行，
/// 前面的大段进度没有价值。超出部分读完就丢 —— 管道照常抽干，子进程不会因为没人读而堵住。
pub const MAX_CHILD_TAIL_BYTES: usize = 64 * 1024;
/// ffprobe 单次探测 stdout 的上限。ffprobe 的 JSON 在**开头**，所以这里不能只留尾部：
/// 超限就**整份拒绝**（半截 JSON 解析不了，比读不到更难查）。正常文件是几 KB 级，
/// 4 MiB 是「真的用得上的最大值再留一倍」之外再留一档。
pub const MAX_FFPROBE_BYTES: u64 = 4 * 1024 * 1024;

/// 超限就拒绝，**不截断**：截出来的文本会静默变成「另一份歌词」，截出来的图片是坏图 ——
/// 比直接读不到更难排查。`Err` 里的句子直接进日志。
pub fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("读不到文件信息：{e}"))?.len();
    if size > cap {
        return Err(format!("体积 {size} 字节，超过上限 {cap}"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
    // 量一次再读一次：中间文件可能长大了（正在下载/正在转码），这时读完的长度才算数
    if bytes.len() as u64 > cap {
        return Err(format!("体积 {} 字节，超过上限 {cap}", bytes.len()));
    }
    Ok(bytes)
}

/// 严格 UTF-8 的文本读取（带上限）。给「按 UTF-8 解析才有意义」的地方用（字幕解析、
/// 译文导出）：这些路径上原来的实现就是 `read_to_string`，编码不对时本来就报错 ——
/// 换成 `from_utf8_lossy` 会把源文本里的坏字节静默变成 U+FFFD，译文和原文对不上号。
pub fn read_text_capped(path: &Path, cap: u64) -> Result<String, String> {
    let bytes = read_capped(path, cap)?;
    String::from_utf8(bytes).map_err(|e| format!("不是 UTF-8 文本：{e}"))
}

/// 解码前先量像素数（只读文件头，不解码）。`None` = 认不出格式。
pub fn image_pixels(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// 只拦「量得出尺寸且确实过大」的图。**认不出格式的一律放行** ——
/// `image` crate 开着的格式比 WebView 窄（webp/avif 之类要按 feature 开），
/// 把「这里认不出」当成「有问题」会挡掉本来能正常显示的封面；而认不出的图我们也不解码它。
pub fn check_image_pixels(bytes: &[u8], cap: u64) -> Result<(), String> {
    match image_pixels(bytes) {
        Some((w, h)) if u64::from(w) * u64::from(h) > cap => {
            Err(format!("图片 {w}×{h}（{} 像素），超过上限 {cap}", u64::from(w) * u64::from(h)))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&vec![0u8; (w as usize) * (h as usize) * 3], w, h, image::ExtendedColorType::Rgb8)
            .expect("png 编码");
        out
    }

    #[test]
    fn read_capped_passes_small_files_through() {
        let p = std::env::temp_dir().join(format!("kx-limits-{}-small.txt", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(read_capped(&p, 1024).expect("小文件放行"), b"abc");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_capped_refuses_instead_of_truncating() {
        let p = std::env::temp_dir().join(format!("kx-limits-{}-big.txt", std::process::id()));
        std::fs::write(&p, vec![b'x'; 4096]).unwrap();
        let e = read_capped(&p, 1024).expect_err("超限必须拒绝");
        assert!(e.contains("4096"), "拒绝语句要带上实际大小：{e}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_capped_reports_missing_files() {
        let p = std::env::temp_dir().join("kx-limits-does-not-exist.txt");
        assert!(read_capped(&p, 1024).is_err());
    }

    /// 文本读取保持严格 UTF-8：坏字节要报错，不能被静默替换成 U+FFFD
    /// （译文和原文对不上号时，比直接失败难查得多）。
    #[test]
    fn text_read_stays_strict_utf8() {
        let ok = std::env::temp_dir().join(format!("kx-limits-{}-utf8.txt", std::process::id()));
        std::fs::write(&ok, "甲\n00:01,000 --> 00:02,000\n乙\n").unwrap();
        assert!(read_text_capped(&ok, 1024).expect("合法 UTF-8").contains("甲"));
        let bad = std::env::temp_dir().join(format!("kx-limits-{}-gbk.txt", std::process::id()));
        std::fs::write(&bad, [0xD6, 0xD0, 0xCE, 0xC4]).unwrap(); // GBK 的「中文」
        let e = read_text_capped(&bad, 1024).expect_err("非 UTF-8 必须报错");
        assert!(e.contains("UTF-8"), "错误语句要点明编码问题：{e}");
        let _ = std::fs::remove_file(&ok);
        let _ = std::fs::remove_file(&bad);
    }

    #[test]
    fn pixel_guard_catches_the_bomb() {
        // 4000×4000 = 1600 万像素：字节数不大，但远超这里给的小上限
        let bytes = png(4000, 4000);
        let e = check_image_pixels(&bytes, 1_000_000).expect_err("超像素必须拒绝");
        assert!(e.contains("16000000"), "拒绝语句要带上实际像素数：{e}");
        assert!(check_image_pixels(&bytes, 20_000_000).is_ok(), "同图在上限够大时放行");
    }

    /// 认不出格式的输入不算超限：`image` crate 能认的格式比 WebView 窄，
    /// 把「这里认不出」判成「有问题」会挡掉本来能显示的封面。
    #[test]
    fn pixel_guard_lets_unknown_formats_through() {
        assert!(image_pixels(b"not an image").is_none());
        assert!(check_image_pixels(b"not an image", 1_000).is_ok());
    }
}
