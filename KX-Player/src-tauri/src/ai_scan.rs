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
/// 收第 0..=max_depth 层的字幕文件（缺省 4 层，硬上限 32）。
/// `limit` 是可选的数量上限，None = **不限量**（用户裁决：扫到多少载入多少 ——
/// 大库也全量进队列与译文管理；显式传值才生效，比如测试里截断用）。
/// 同层条目**排序后**返回：readdir 的顺序两个平台都不保证，旧实现直接吃目录顺序，
/// 同一批文件两次扫描能给出不同的队列。遍历是按名排序的深度优先。
///
/// 曾经还有一个 `ScanKind::Dir`（列子目录给「文件夹名翻译」排队用）。那条链第 21 批起
/// 整体并进了改名主干（`rename/listing::preview_roots` 一层扫描 + `rename_name_cache`
/// 内容键账），这里不留第二套：一个永远走不到的 kind 会让 `translated` 的注释开始撒谎。
pub fn scan_dir(root: &str, max_depth: usize, limit: Option<usize>) -> Vec<ScannedItem> {
    let mut out: Vec<ScannedItem> = Vec::new();
    walk(root, 0, max_depth.min(32), limit, &mut out);
    out
}

fn walk(dir: &str, depth: usize, max_depth: usize, limit: Option<usize>, out: &mut Vec<ScannedItem>) {
    if depth > max_depth || limit.is_some_and(|l| out.len() >= l) {
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
        if limit.is_some_and(|l| out.len() >= l) {
            return;
        }
        let path = prompt::join_path(dir, &name);
        if is_dir {
            walk(&path, depth + 1, max_depth, limit, out);
        } else {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            // 译文文件（1.lrc → 1.zh.lrc）不进队列：它们是上一轮的产物，再扫进来会被
            // 当新字幕翻出 1.zh.zh.*。这里让它们根本不出现在扫描结果里；翻译管线入口
            // （ai_file）另有兜底，旧扫描结果不重扫也递不进来。
            if subs::SUBTITLE_EXTS.contains(&ext.as_str()) && !prompt::is_translated_output(&name) {
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
        let items = scan_dir(root.to_str().unwrap(), 4, None);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["a.lrc", "b.SRT", "d.vtt"], "非字幕扩展名不进队列");
        assert_eq!(items[1].ext.as_deref(), Some("srt"), "扩展名统一小写");
        assert!(items[2].path.ends_with("/sub/d.vtt"), "子目录递归：{}", items[2].path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn translated_outputs_never_enter_the_queue() {
        let root = tmpdir("zh");
        std::fs::write(root.join("1.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(root.join("1.zh.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(root.join("1.zh.zh.srt"), "1").unwrap();
        std::fs::write(root.join("song.zhang.lrc"), "x").unwrap();
        let names: Vec<String> = scan_dir(root.to_str().unwrap(), 4, None).into_iter().map(|i| i.name).collect();
        assert_eq!(
            names,
            vec!["1.lrc", "song.zhang.lrc"],
            "译文文件不进扫描结果（否则被再翻一遍）；名字里恰好含 zh 的源文件照常进"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_is_unbounded_by_default_and_explicit_limit_still_works() {
        let root = tmpdir("unbounded");
        for i in 0..600 {
            std::fs::write(root.join(format!("s{i:04}.lrc")), "x").unwrap();
        }
        assert_eq!(scan_dir(root.to_str().unwrap(), 4, None).len(), 600, "缺省不限量：扫到多少载入多少");
        assert_eq!(scan_dir(root.to_str().unwrap(), 4, Some(500)).len(), 500, "显式上限仍然生效");
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
        assert_eq!(names(scan_dir(root.to_str().unwrap(), 0, None)), vec!["deep.lrc"]);
        assert_eq!(
            names(scan_dir(root.to_str().unwrap(), 1, None)),
            vec!["d1.lrc", "deep.lrc"],
            "按名排序的深度优先：a/ 排在 deep.lrc 前面，先递归进去"
        );
        assert_eq!(scan_dir(root.to_str().unwrap(), 3, Some(1)).len(), 1, "显式上限先到就停");
        let _ = std::fs::remove_dir_all(&root);
    }
}
