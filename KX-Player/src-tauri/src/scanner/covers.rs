//! 封面子系统：压缩落盘 + 三级查找（缓存 → 外部文件 → 内嵌）+ 文件夹封面映射。
//! 与 Electron coverService/fileScanner 的行为逐条对齐；封面以「文件路径」形式流转，
//! 不再把 base64 常驻内存。

use super::meta::{image_dimensions, ext_of};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const COVER_FILE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];
const MAX_COVER_BYTES: usize = 15 * 1024 * 1024;
const COVER_NAME_HINTS: &[&str] = &[
    "cover", "folder", "front", "albumart", "album", "art", "jacket", "ジャケット", "封面", "专辑封面", "专辑图",
];
const NON_COVER_HINTS: &[&str] = &[
    "ui", "说明", "screenshot", "screen", "manual", "readme", "player", "capture", "shot", "banner", "icon",
    "thumb", "thumbnail", "small",
];
const MAX_COVER_DIM: u32 = 300;

static FOLDER_COVER_MAP: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(s.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn covers_dir() -> PathBuf {
    crate::paths::covers_dir()
}

/// 压缩为 ≤300px JPEG q70；<10KB 原样保留（与 compressToJpeg 一致）
pub fn compress_to_jpeg(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() < 10 * 1024 {
        return Some(input.to_vec());
    }
    let img = image::load_from_memory(input).ok()?;
    let resized = if img.width() > MAX_COVER_DIM || img.height() > MAX_COVER_DIM {
        img.resize(MAX_COVER_DIM, MAX_COVER_DIM, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70);
    enc.encode_image(&resized).ok()?;
    Some(out)
}

/// 把封面字节写入 covers/{name}.jpg，返回绝对路径
pub fn save_cover_bytes(name: &str, bytes: &[u8]) -> Option<String> {
    let compressed = compress_to_jpeg(bytes)?;
    let path = covers_dir().join(format!("{name}.jpg"));
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(&compressed).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// 把封面文件压缩后写入 covers/{name}.jpg（外部图片文件 → 封面）
pub fn save_cover_file(name: &str, src: &Path) -> Option<String> {
    let bytes = std::fs::read(src).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    save_cover_bytes(name, &bytes)
}

pub fn track_cover_path(track_id: &str) -> Option<String> {
    let p = covers_dir().join(format!("{track_id}.jpg"));
    p.exists().then(|| p.to_string_lossy().into_owned())
}

pub fn copy_track_cover(from_id: &str, to_id: &str) -> bool {
    let src = covers_dir().join(format!("{from_id}.jpg"));
    let dst = covers_dir().join(format!("{to_id}.jpg"));
    if !src.exists() {
        return false;
    }
    std::fs::copy(&src, &dst).is_ok()
}

// ── 外部封面查找与打分 ──────────────────────────────────────────

fn is_cover_file(name: &str) -> bool {
    COVER_FILE_EXTS.contains(&ext_of(name).as_str())
}

fn score_cover_candidate(path: &Path, depth: i32) -> i32 {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = ext_of(&name);
    let size_kb = std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0);

    for hint in NON_COVER_HINTS {
        if name.contains(hint) {
            return -1000;
        }
    }
    let mut score = 0;
    for hint in COVER_NAME_HINTS {
        if name.contains(hint) {
            score += 100;
        }
    }
    if ext == "jpg" || ext == "jpeg" {
        score += 10;
    } else if ext == "png" {
        score += 5;
    }
    score -= depth * 30;

    if let Some((w, h)) = image_dimensions(path) {
        if h == 0 {
            return score;
        }
        let ratio = w as f32 / h as f32;
        if (1.2..=1.5).contains(&ratio) {
            score += 60;
        } else if (0.9..=1.1).contains(&ratio) {
            score += 40;
        } else if ratio > 2.5 || ratio < 0.4 {
            score -= 50;
        }
        let long_side = w.max(h);
        if (400..=1200).contains(&long_side) {
            score += 20;
        } else if long_side > 1600 {
            score -= 20;
        }
    } else if (30..=600).contains(&size_kb) {
        score += 10;
    } else if size_kb > 1000 {
        score -= 10;
    }
    score
}

/// 在目录（含子目录 maxDepth 层）内找最佳外部封面图片，返回其路径
pub fn find_external_cover(dir: &str, max_depth: i32) -> Option<PathBuf> {
    let mut best: Option<(i32, PathBuf)> = None;
    fn scan(dir: &str, depth: i32, max_depth: i32, best: &mut Option<(i32, PathBuf)>) {
        if depth > max_depth {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(s) = p.to_str() {
                    scan(s, depth + 1, max_depth, best);
                }
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !is_cover_file(&name) {
                continue;
            }
            let score = score_cover_candidate(&p, depth);
            if best.is_none() || score > best.as_ref().unwrap().0 {
                *best = Some((score, p));
            }
        }
    }
    scan(dir, 0, max_depth, &mut best);
    let (score, path) = best?;
    if score < 0 {
        return None;
    }
    Some(path)
}

/// 目录内任意图片（排除非常规封面），返回路径
pub fn find_any_image(dir: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(i32, PathBuf)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_cover_file(&name) {
            continue;
        }
        let score = score_cover_candidate(&p, 0);
        if score > -100 && (best.is_none() || score > best.as_ref().unwrap().0) {
            best = Some((score, p));
        }
    }
    best.map(|(_, p)| p)
}

// ── 文件夹封面映射（folder-cover-map.json）──────────────────────

fn folder_map_path() -> PathBuf {
    crate::paths::data_dir().join("folder-cover-map.json")
}

pub fn load_folder_cover_map() {
    let mut guard = FOLDER_COVER_MAP.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(
        std::fs::read_to_string(folder_map_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    );
}

fn persist_folder_cover_map(map: &HashMap<String, String>) {
    if let Ok(json) = serde_json::to_string(map) {
        let _ = std::fs::write(folder_map_path(), json);
    }
}

/// 记录目录 → 封面文件映射并落盘
pub fn set_folder_cover_mapping(folder_path: &str) {
    let mut guard = FOLDER_COVER_MAP.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(folder_path.to_string(), md5_hex(folder_path));
    persist_folder_cover_map(map);
}

pub fn folder_cover_path(folder_path: &str) -> Option<String> {
    let hash = {
        let guard = FOLDER_COVER_MAP.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref()?.get(folder_path).cloned()
    }?;
    let p = covers_dir().join(format!("folder_{hash}.jpg"));
    p.exists().then(|| p.to_string_lossy().into_owned())
}

pub fn all_folder_cover_paths() -> Vec<(String, String)> {
    let guard = FOLDER_COVER_MAP.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .map(|m| {
            m.iter()
                .filter_map(|(fp, hash)| {
                    let p = covers_dir().join(format!("folder_{hash}.jpg"));
                    p.exists().then(|| (fp.clone(), p.to_string_lossy().into_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 目录存在外部封面时直接以外部文件为封面源（供 saveCovers 兜底）
pub fn external_cover_in_dir(dir: &str) -> Option<PathBuf> {
    find_external_cover(dir, 0).or_else(|| find_any_image(dir))
}

/// 列出 covers 目录中已有的 track 封面 id（Tier1 缓存检查）
pub fn existing_track_cover_ids() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(entries) = std::fs::read_dir(covers_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("folder_") || !name.ends_with(".jpg") {
                continue;
            }
            set.insert(name.trim_end_matches(".jpg").to_string());
        }
    }
    set
}
