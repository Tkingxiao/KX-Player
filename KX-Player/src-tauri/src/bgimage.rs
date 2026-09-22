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
    let luma = std::fs::read(&p).ok().and_then(|bytes| sample_luma(&bytes));
    Some(BgImageData { path: p.to_string_lossy().into_owned(), mtime: Some(ms), luma })
}

/// 存 base64 data URL（前端 BgEditor 剪裁后的输出）
pub fn save_data_url(data_url: &str) -> bool {
    let Some(b64) = data_url.split(',').next_back() else { return false };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return false };
    write_bytes(&bytes)
}

/// 选择新背景图：压缩落盘，回读路径 + mtime + 感知亮度
pub fn select_and_save(src: &str) -> Option<BgImageData> {
    let bytes = std::fs::read(src).ok()?;
    if write_bytes(&bytes) {
        load()
    } else {
        None
    }
}

fn write_bytes(bytes: &[u8]) -> bool {
    let p = bg_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match resize_to_jpeg(bytes) {
        Some(out) => {
            let tmp = p.with_extension("png.tmp");
            if std::fs::write(&tmp, &out).is_ok() {
                std::fs::rename(&tmp, &p).is_ok()
            } else {
                std::fs::write(&p, &out).is_ok()
            }
        }
        None => std::fs::write(&p, bytes).is_ok(),
    }
}

pub fn remove() -> bool {
    let p = bg_path();
    if p.exists() {
        std::fs::remove_file(&p).is_ok()
    } else {
        true
    }
}

/// 迁移：旧版超大壁纸压缩（启动时调用一次）
pub fn migrate_oversized() {
    let p = bg_path();
    if !p.exists() {
        return;
    }
    let Ok(bytes) = std::fs::read(&p) else { return };
    let dim = image::load_from_memory(&bytes).ok().map(|i| (i.width(), i.height()));
    if let Some((w, h)) = dim {
        if w <= MAX_BG_DIM && h <= MAX_BG_DIM {
            return;
        }
        if let Some(out) = resize_to_jpeg(&bytes) {
            let tmp = p.with_extension("png.tmp");
            if std::fs::write(&tmp, &out).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
    let _ = Path::new(&p).exists();
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
