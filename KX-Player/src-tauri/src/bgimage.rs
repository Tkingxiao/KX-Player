//! 背景图：选择/保存/移除/加载（≤2560px JPEG q82，与 Electron 版一致）。

use crate::model::BgImageData;
use base64::Engine;
use image::GenericImageView as _;
use std::path::Path;

const MAX_BG_DIM: u32 = 2560;

fn resize_to_jpeg(input: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(input).ok()?;
    let resized = if img.width() > MAX_BG_DIM || img.height() > MAX_BG_DIM {
        img.resize(MAX_BG_DIM, MAX_BG_DIM, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82);
    enc.encode_image(&resized).ok()?;
    Some(out)
}

fn bg_path() -> std::path::PathBuf {
    crate::paths::bg_image_path()
}

/// rec.601 感知亮度（0–255）。与前端 `utils/color.ts::perceivedLuma` 同式 ——
/// 跨语言无法共享常量，改一处必须同步另一处，否则阈值 128 会在两侧含义不同。
pub fn luma_of_rgb(r: u8, g: u8, b: u8) -> f64 {
    0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
}

/// 背景图的平均感知亮度：缩到 50×50 再逐像素加权（P0-68 亮度自适应的输入）。
/// 在 Rust 侧算而非前端 canvas：图片走 `asset:` 协议，与页面跨源，canvas 会被 taint，
/// `getImageData` 必然抛 SecurityError —— 这正是旧版「明度自适应」恒判为深底的原因。
pub fn sample_luma(bytes: &[u8]) -> Option<f64> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.resize(50, 50, image::imageops::FilterType::Triangle);
    let n = small.width() * small.height();
    if n == 0 {
        return None;
    }
    let sum: f64 = small.pixels().map(|(_, _, p)| luma_of_rgb(p[0], p[1], p[2])).sum();
    Some(sum / f64::from(n))
}

pub fn load() -> Option<BgImageData> {
    let p = bg_path();
    if !p.exists() {
        return None;
    }
    let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
    let ms = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as f64;
    // 自产文件的量级本该远小于图片上限；按上限读只是不让「外来/损坏的旧文件」整份进内存。
    // 超了就当取不到亮度，背景图本身照常显示。
    let luma = crate::limits::read_capped(&p, crate::limits::MAX_IMAGE_BYTES).ok().and_then(|bytes| sample_luma(&bytes));
    Some(BgImageData { path: p.to_string_lossy().into_owned(), mtime: Some(ms), luma })
}

/// 存 base64 data URL（前端 BgEditor 剪裁后的输出）
pub fn save_data_url(data_url: &str) -> bool {
    let Some(b64) = data_url.split(',').next_back() else { return false };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return false };
    // 先按 base64 解出来的长度拦一道：几百 MB 的串在前端拼得出来，解完才发现太大就晚了
    if bytes.len() as u64 > crate::limits::MAX_IMAGE_BYTES {
        crate::paths::append_log(&format!(
            "[limit] 背景图拒绝（解码后 {} 字节，超过上限 {}）",
            bytes.len(),
            crate::limits::MAX_IMAGE_BYTES
        ));
        return false;
    }
    write_bytes(&bytes)
}

/// 选择新背景图：压缩落盘，回读路径 + mtime + 感知亮度
pub fn select_and_save(src: &str) -> Option<BgImageData> {
    let bytes = match crate::limits::read_capped(Path::new(src), crate::limits::MAX_IMAGE_BYTES) {
        Ok(b) => b,
        Err(why) => {
            crate::paths::append_log(&format!("[limit] 选择背景图拒绝 {src}（{why}）"));
            return None;
        }
    };
    if write_bytes(&bytes) {
        load()
    } else {
        None
    }
}

fn write_bytes(bytes: &[u8]) -> bool {
    // 压缩（`resize_to_jpeg` 会真解码）之前先把像素量出来：几十 KB 的 PNG 能解出上亿像素
    if let Err(why) = crate::limits::check_image_pixels(bytes, crate::limits::MAX_IMAGE_PIXELS) {
        crate::paths::append_log(&format!("[limit] 背景图拒绝（{why}）"));
        return false;
    }
    let p = bg_path();
    let out = match resize_to_jpeg(bytes) {
        Some(out) => out,
        None => bytes.to_vec(),
    };
    crate::atomic::write(&p, &out).is_ok()
}

pub fn remove() -> bool {
    let p = bg_path();
    if p.exists() {
        std::fs::remove_file(&p).is_ok()
    } else {
        true
    }
}

/// 迁移读取的上限：这条路径读的**正是**「旧版留下的超大壁纸」，不能按 `MAX_IMAGE_BYTES`
/// 拦（那正是要处理的对象）；这里只防「意外的大文件」被整份读进来 —— 再大的图片也不是
/// 壁纸，跳过迁移比读进内存强。
const MIGRATE_MAX_READ: u64 = 256 * 1024 * 1024;

/// 迁移：旧版超大壁纸压缩（启动时调用一次）
pub fn migrate_oversized() {
    let p = bg_path();
    if !p.exists() {
        return;
    }
    let bytes = match crate::limits::read_capped(&p, MIGRATE_MAX_READ) {
        Ok(b) => b,
        Err(e) => {
            // 跳过要说一声：否则「旧壁纸一直没被压小」看起来像迁移坏了
            crate::paths::append_log(&format!("[limit] 跳过背景图迁移：{e}"));
            return;
        }
    };
    let dim = image::load_from_memory(&bytes).ok().map(|i| (i.width(), i.height()));
    if let Some((w, h)) = dim {
        if w <= MAX_BG_DIM && h <= MAX_BG_DIM {
            return;
        }
        if let Some(out) = resize_to_jpeg(&bytes) {
            let _ = crate::atomic::write(&p, &out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn png(bytes: &[u8], w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(bytes, w, h, image::ExtendedColorType::Rgb8)
            .expect("png 编码");
        out
    }

    #[test]
    fn luma_follows_rec601_weights() {
        assert_eq!(luma_of_rgb(0, 0, 0), 0.0);
        assert!((luma_of_rgb(255, 255, 255) - 255.0).abs() < 0.01);
        // 绿色权重最高：同一个 200 灰阶，纯绿比纯蓝亮得多
        assert!(luma_of_rgb(0, 200, 0) > luma_of_rgb(0, 0, 200));
        assert!((luma_of_rgb(0, 200, 0) - 117.4).abs() < 0.1);
    }

    #[test]
    fn sample_luma_of_a_flat_image_is_that_color() {
        let one = [30u8, 240, 90];
        let bytes = png(&one.repeat(64), 8, 8);
        let got = sample_luma(&bytes).expect("可平图可采样");
        assert!((got - luma_of_rgb(one[0], one[1], one[2])).abs() < 1.0, "平图均值应等于该色亮度，实测 {got}");
    }

    #[test]
    fn sample_luma_sees_the_darker_half_of_a_split_image() {
        let mut px = Vec::new();
        for y in 0..8 {
            for _ in 0..8 {
                px.extend_from_slice(if y < 4 { &[255, 255, 255] } else { &[0, 0, 0] });
            }
        }
        let got = sample_luma(&png(&px, 8, 8)).expect("可分界图可采样");
        assert!((got - 127.5).abs() < 1.0, "上下各半应接近 127.5，实测 {got}");
    }

    #[test]
    fn sample_luma_returns_none_for_non_images() {
        assert!(sample_luma(b"not an image at all").is_none());
        assert!(sample_luma(&[]).is_none());
    }
}
