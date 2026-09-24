//! AI 翻译页的目录扫描（B3）：一次 IPC 调用替代旧前端的 `api.listDir` 逐层递归。
//!
//! 从 `ai_job.rs` 分出来是为了守住 `01 §5-20` 的 Rust ≤400 行；两者只共享一个约定：
//! 扫描只负责**列出路径**，任务只认路径 —— 扫描不知道模型、任务不知道目录树。

use crate::ai_prompt as prompt;
use crate::subtitles as subs;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedItem {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    /// 是否已翻译可豁免（字幕：上级账命中且译文文件还在）。扫描命令回填，默认 false。
    #[serde(skip_serializing_if = "not_translated")]
    pub translated: bool,
}

/// serde 的 skip_serializing_if 需要一个真值只出现一次的长寿命命中；这里用它把 false 从载荷里省掉，
/// 老前端不认识这个新字段也不受影响（收到 false 时行为不变）。
fn not_translated(b: &bool) -> bool {
    !*b
}

impl ScannedItem {
    pub fn new(path: String, name: String, ext: Option<String>) -> Self {
        Self { path, name, ext, translated: false }
    }
}

/// 递归扫描字幕文件。max_depth 是**相对根目录的深度**，根目录里的条目算第 0 层：
/// 收第 0..=max_depth 层的字幕文件（旧前端传 4、上限 500）。
/// 同层条目**排序后**返回：readdir 的顺序两个平台都不保证，旧实现直接吃目录顺序，
/// 同一批文件两次扫描能给出不同的队列。遍历是按名排序的深度优先，所以 limit 截断时
/// 留下哪一批也由这个名字顺序决定 —— 两次扫描的队列一致，这才是要点。
///
/// 曾经还有一个 `ScanKind::Dir`（列子目录给「文件夹名翻译」排队用）。那条链第 21 批起
/// 整体并进了改名主干（`rename/listing::preview_roots` 一层扫描 + `rename_name_cache`
/// 内容键账），这里不留第二套：一个永远走不到的 kind 会让 `translated` 的注释开始撒谎。
pub fn scan_dir(root: &str, max_depth: usize, limit: usize) -> Vec<ScannedItem> {
    let mut out: Vec<ScannedItem> = Vec::new();
    walk(root, 0, max_depth.min(32), limit.min(10_000), &mut out);
    out
}

fn walk(dir: &str, depth: usize, max_depth: usize, limit: usize, out: &mut Vec<ScannedItem>) {
    if out.len() >= limit || depth > max_depth {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, bool)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().ok()?.is_dir();
            Some((name, is_dir))
        })
        .collect();
    entries.sort();
    for (name, is_dir) in entries {
        if out.len() >= limit {
            return;
        }
        let path = prompt::join_path(dir, &name);
        if is_dir {
            walk(&path, depth + 1, max_depth, limit, out);
        } else {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            if subs::SUBTITLE_EXTS.contains(&ext.as_str()) {
                out.push(ScannedItem::new(path, name, Some(ext)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("kx-scan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scan_finds_subtitles_and_skips_other_files() {
        let root = tmpdir("subs");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(root.join("b.SRT"), "1").unwrap();
        std::fs::write(root.join("c.txt"), "nope").unwrap();
        std::fs::write(root.join("sub").join("d.vtt"), "WEBVTT").unwrap();
        let items = scan_dir(root.to_str().unwrap(), 4, 500);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["a.lrc", "b.SRT", "d.vtt"], "非字幕扩展名不进队列");
        assert_eq!(items[1].ext.as_deref(), Some("srt"), "扩展名统一小写");
        assert!(items[2].path.ends_with("/sub/d.vtt"), "子目录递归：{}", items[2].path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_respects_depth_and_limit_as_separate_bounds() {
        let root = tmpdir("bounds");
        std::fs::create_dir_all(root.join("a").join("b").join("c")).unwrap();
        std::fs::write(root.join("deep.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(root.join("a").join("d1.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(root.join("a").join("b").join("c").join("d2.lrc"), "[00:01.00]x").unwrap();
        let names = |items: Vec<ScannedItem>| items.into_iter().map(|i| i.name).collect::<Vec<_>>();
        assert_eq!(names(scan_dir(root.to_str().unwrap(), 0, 500)), vec!["deep.lrc"]);
        assert_eq!(
            names(scan_dir(root.to_str().unwrap(), 1, 500)),
            vec!["d1.lrc", "deep.lrc"],
            "按名排序的深度优先：a/ 排在 deep.lrc 前面，先递归进去"
        );
        assert_eq!(scan_dir(root.to_str().unwrap(), 3, 1).len(), 1, "limit 先到就停");
        let _ = std::fs::remove_dir_all(&root);
    }
}
