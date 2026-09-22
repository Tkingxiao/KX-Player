//! 把一层一层的预览**凑成一次预览**：列目录（唯一读盘的一处）+ 汇总计数 + 截断。
//!
//! 与 `preview.rs` 的分界就是「要不要文件系统」：那半边全是纯函数，能拿名字列表写用例；
//! 这半边只有 `read_dir`，能测的只有顺序与失败上报。

use std::path::Path;

use crate::rename::preview::{preview_level, Counts, Entry, Item, Preview};

/// 一次预览最多回多少条：一层的作品目录上千是常态，整包塞进 IPC 只为了让人看见前几条没意义。
/// `counts` 永远是全量的，被截断时 `truncated=true`（`04 §7.2` 的分布判据不能是半份数据）。
pub const MAX_ITEMS: usize = 300;

/// `roots` 是用户勾出来的目录，只往下看**一层** —— 根目录本身不动，它就是用户划下的边界
/// （`04 §7.1` 第 5 条：其它根目录要显式勾选才纳入）。
pub fn preview_roots(roots: &[String]) -> Preview {
    let mut out = Preview::default();
    for root in roots {
        let Some(entries) = list_one_level(root) else {
            out.unreadable.push(root.clone());
            continue;
        };
        let folder_name = Path::new(root).file_name().map(|n| n.to_string_lossy().into_owned());
        let (items, counts) = preview_level(root, folder_name.as_deref(), &entries);
        merge(&mut out, counts, items);
    }
    out
}

fn merge(out: &mut Preview, n: Counts, items: Vec<Item>) {
    let left = MAX_ITEMS.saturating_sub(out.items.len());
    if items.len() > left {
        out.truncated = true;
    }
    out.items.extend(items.into_iter().take(left));
    let c = &mut out.counts;
    c.scanned += n.scanned;
    c.skip += n.skip;
    c.normalize_only += n.normalize_only;
    c.needs_repair += n.needs_repair;
    c.needs_translation += n.needs_translation;
    c.compliant += n.compliant;
    c.changed += n.changed;
    c.conflicts += n.conflicts;
}

/// 列一层：目录在前、文件在后，各自按名字排序 —— `read_dir` 不保证顺序，两次预览必须是同一份队列
fn list_one_level(root: &str) -> Option<Vec<Entry>> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut entries: Vec<Entry> = rd
        .flatten()
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().into_owned();
            let is_dir = de.file_type().ok()?.is_dir();
            Some(Entry { name, is_dir })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("kx-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_roots_are_reported_instead_of_looking_empty() {
        let p = preview_roots(&["L:/肯定不存在的路径".into()]);
        assert_eq!(p.unreadable.len(), 1);
        assert_eq!(p.counts.scanned, 0, "列不出来 ≠ 这层是空的");
    }

    /// 只有这一层是真读盘的：路径拼接、根目录本身不进队列、一个根失败不拖累别的根，
    /// 这三件事在纯函数那半边测不出来（那边收的是名字列表）。
    #[test]
    fn roots_are_read_one_level_and_paths_point_back_at_the_file() {
        let base = tmpdir("roots");
        let root = base.join("作品 第一回");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join("特典~")).unwrap();
        std::fs::write(root.join("歌  手.srt"), "1").unwrap();
        std::fs::write(root.join("正常名字.lrc"), "1").unwrap();
        let root_str = root.to_string_lossy().into_owned();

        let p = preview_roots(std::slice::from_ref(&root_str));
        assert_eq!(p.counts.scanned, 3, "根目录自己不是一条");
        assert_eq!(p.counts.compliant, 1, "已合规的那条不列出来，但账要记");
        assert_eq!(
            p.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["特典~", "歌  手.srt"],
            "目录在前、各自按名字排"
        );
        for i in &p.items {
            assert!(i.path.starts_with(&root_str), "path 要能回指原文件：{}", i.path);
            assert!(i.changed, "列出来的这两条都该是真有字形变化");
        }

        let both = preview_roots(&[root_str, "L:/没有这个地方".into()]);
        assert_eq!(both.unreadable.len(), 1);
        assert_eq!(both.counts.scanned, 3, "一个根列不出来，不能把另一个根的账一起丢掉");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn folders_come_before_files_so_the_preview_reads_in_apply_order() {
        // 应用顺序是自底向上的文件夹先、文件后（`04 §7.8`），预览列出的顺序要跟它一致
        let listed = list_one_level(".").expect("当前目录该列得出来");
        assert!(
            listed.windows(2).all(|w| w[0].is_dir || !w[1].is_dir),
            "列一层时目录必须在前"
        );
    }
}
