//! 背景图：选择/保存/移除/加载（≤2560px JPEG q82，与 Electron 版一致）。

use crate::model::BgImageData;
use base64::Engine;
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

pub fn load() -> Option<BgImageData> {
    let p = bg_path();
    if !p.exists() {
        return None;
    }
    let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
    let ms = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as f64;
    Some(BgImageData { path: p.to_string_lossy().into_owned(), mtime: Some(ms) })
}

/// 存 base64 data URL（前端 BgEditor 剪裁后的输出）
pub fn save_data_url(data_url: &str) -> bool {
    let Some(b64) = data_url.split(',').next_back() else { return false };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return false };
    write_bytes(&bytes)
}

/// 选择新背景图：压缩落盘，返回路径 + mtime
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
