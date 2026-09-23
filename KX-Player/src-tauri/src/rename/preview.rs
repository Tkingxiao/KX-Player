//! 改名**预览**（`04 §7.8` 只读的那一半）：一层目录的条目名 → 判定 → 本地规范化 → 找冲突。
//!
//! 这一份全是纯函数（连文件名都是字符串，不碰文件系统），「分布」与「冲突」这两件事因此
//! 写得动用例 —— 批量改名没有第二次机会，判错的代价由用例来兜。列目录与汇总在 `listing.rs`。
//! 列出的顺序与将来应用的顺序一致：文件夹在前、文件在后，各自按名字排（`04 §7.8`）。

use std::path::Path;

use crate::rename::judge::{judge, Ctx, Judgment, Reason, Verdict};
use crate::rename::norm::{self, Family, FILE_MAX, FOLDER_MAX, PATH_MAX};

/// 目录里的一条待判名条目
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Folder,
    File,
}

/// 冲突（`04 §7.8` 那张表）。命中任何一条都不能自动改，只能报给用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    /// 批内有另外一条也要改成这个名字
    SameTarget,
    /// 目标名在这层目录里已被别的条目占着
    TargetExists,
    /// 改完的全路径超过 `PATH_MAX`
    PathTooLong,
}

/// 一条预览结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// 原绝对路径：后续应用/回滚都以它为键
    pub path: String,
    /// 当前名字（带扩展名，用户看得懂的那个）
    pub name: String,
    /// 本地规范化之后的名字。NEEDS_* 档这条只是**进模型之前**的形态，不是最终名
    pub target: String,
    pub kind: Kind,
    pub verdict: Verdict,
    pub reasons: Vec<Reason>,
    /// 档位之外还真有字面变化：NORMALIZE_ONLY 也可能一条都没改着（只剩保留字符）
    pub changed: bool,
    /// 还需要模型接着做（`04 §7.6` 的入口判据）
    pub needs_ai: bool,
    /// **库里已核准的那个译名**（`rename_name_cache`，过了九道校验的才有）。
    /// `preview_level` 是纯函数所以恒为 `None`；由 `ai_ledger::annotate` 在读库之后盖上 ——
    /// 面板据此说「这一批能应用几条」，而不是让用户猜「生成译名」那一步做没做。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
}

/// 档位分布。计数含没变化的条目，否则「COMPLIANT 占几成」算不出来。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub scanned: usize,
    pub skip: usize,
    pub normalize_only: usize,
    pub needs_repair: usize,
    pub needs_translation: usize,
    pub compliant: usize,
    pub changed: usize,
    pub conflicts: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub counts: Counts,
    /// 有变化、有冲突，或档位不是 COMPLIANT 的条目；三者都不占的（绝大多数）不列出
    pub items: Vec<Item>,
    pub truncated: bool,
    /// 列不出来的目录（不存在 / 没权限），前端要如实显示而不是当成空
    pub unreadable: Vec<String>,
}

/// 一个条目算出来的东西：规范化后的主体、拼回扩展名的整名、有没有真变
#[derive(Debug, Clone)]
struct Row {
    stem_target: String,
    target: String,
    changed: bool,
}

/// 纯函数：同一层目录里的一批条目 → 每条的判定 + 规范化名 + 冲突。
///
/// `dir_path` 只用来拼全路径做长度检查；`folder_name` 是这一层目录自己的名字，
/// 只给文件条目用（`duplicate_title` 说的是「文件名里重复了所属作品名」）。
pub fn preview_level(
    dir_path: &str,
    folder_name: Option<&str>,
    entries: &[Entry],
) -> (Vec<Item>, Counts) {
    // 同目录内前缀族 >1 种 → prefix_mixed（`04 §7.2` 第 8 条），要先扫完一层才知道
    let mut families: Vec<Family> = Vec::new();
    for e in entries {
        let f = norm::family(norm::split_prefix(stem(e)).prefix);
        if f != Family::None && !families.contains(&f) {
            families.push(f);
        }
    }
    let mixed = families.len() > 1;
    let rows: Vec<Row> = entries.iter().map(normalize_entry).collect();

    let mut counts = Counts::default();
    let mut items: Vec<Item> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let ctx = ctx_for(e, folder_name, mixed);
        let j = judge(stem(e), max_for(e), &ctx);
        let row = &rows[i];
        // 没在动的条目不可能有冲突要报（它占着的那个名字是别人的冲突，不是它的）
        let conflict = if row.changed {
            find_conflict(dir_path, i, e, row, entries, &rows)
        } else {
            None
        };
        counts.bump(&j, row.changed, conflict.is_some());
        // COMPLIANT 且没变没冲突的（绝大多数）不打扰用户；其余都列出来，
        // 包括「只报告不改」的 prefix_mixed —— 它要的就是让用户看见（`04 §7.2` 第 8 条）
        if j.verdict == Verdict::Compliant && !row.changed && conflict.is_none() {
            continue;
        }
        // 档位按 §7.2 对**原名**的第一命中定；要不要进模型看**规范化之后**还剩什么 ——
        // 规则能修掉的不该花 token（§7.11），而 `名称~` 这种「符号＋假名」的条目，
        // 原名第一命中是 wave，把波浪号收掉之后假名才露出来，不重判就会永久停在本地档。
        let after = judge(&row.stem_target, max_for(e), &ctx_for(e, folder_name, mixed));
        items.push(Item {
            path: Path::new(dir_path).join(&e.name).to_string_lossy().into_owned(),
            name: e.name.clone(),
            target: row.target.clone(),
            kind: if e.is_dir { Kind::Folder } else { Kind::File },
            verdict: j.verdict,
            needs_ai: after.needs_ai(),
            suggested: None,
            reasons: j.reasons,
            changed: row.changed,
            conflict,
        });
    }
    (items, counts)
}

impl Counts {
    fn bump(&mut self, j: &Judgment, changed: bool, conflict: bool) {
        self.scanned += 1;
        match j.verdict {
            Verdict::Skip => self.skip += 1,
            Verdict::NormalizeOnly => self.normalize_only += 1,
            Verdict::NeedsRepair => self.needs_repair += 1,
            Verdict::NeedsTranslation => self.needs_translation += 1,
            Verdict::Compliant => self.compliant += 1,
        }
        if changed {
            self.changed += 1;
        }
        if conflict {
            self.conflicts += 1;
        }
    }
}

/// 本地规范化后的整名（含拼回的扩展名）+ 是否真变了
fn normalize_entry(e: &Entry) -> Row {
    let (stem_name, ext) = split_ext(&e.name);
    let stem_target = norm::normalize(stem_name, max_for(e));
    let target = format!("{stem_target}{ext}");
    Row { changed: target != e.name, stem_target, target }
}

/// 单条判定的上下文。锁定/拒绝写在 `ai_rename_state.json`（`04 §7.1` 第 3 条），本批还没接，恒 None
fn ctx_for<'a>(e: &Entry, folder_name: Option<&'a str>, mixed: bool) -> Ctx<'a> {
    Ctx {
        skipped: None,
        folder: if e.is_dir { None } else { folder_name },
        prefix_mixed: mixed,
    }
}

fn max_for(e: &Entry) -> usize {
    if e.is_dir {
        FOLDER_MAX
    } else {
        FILE_MAX
    }
}

fn find_conflict(
    dir_path: &str,
    idx: usize,
    e: &Entry,
    row: &Row,
    entries: &[Entry],
    rows: &[Row],
) -> Option<Conflict> {
    if row.target.is_empty() {
        return Some(Conflict::TargetExists);
    }
    if entries.iter().zip(rows).enumerate().any(|(k, (o, r))| {
        k != idx && o.is_dir == e.is_dir && r.changed && r.target == row.target
    }) {
        return Some(Conflict::SameTarget);
    }
    if entries
        .iter()
        .enumerate()
        .any(|(k, o)| k != idx && o.is_dir == e.is_dir && o.name == row.target)
    {
        return Some(Conflict::TargetExists);
    }
    if Path::new(dir_path).join(&row.target).to_string_lossy().chars().count() > PATH_MAX {
        return Some(Conflict::PathTooLong);
    }
    None
}

/// 扩展名不参与判定（`04 §7.2` 第 2 条），也永不被改：`.mp3` 原样拼回末尾。
/// 以点开头的名字（`.gitignore`）整个算主体。
fn split_ext(name: &str) -> (&str, &str) {
    match Path::new(name).extension() {
        Some(x) => {
            let x = x.to_string_lossy();
            let cut = name.len() - x.len() - 1;
            (&name[..cut], &name[cut..])
        }
        None => (name, ""),
    }
}

/// 条目的判定用名（文件剥掉扩展名）
fn stem(e: &Entry) -> &str {
    if e.is_dir {
        &e.name
    } else {
        split_ext(&e.name).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> Entry {
        Entry { name: name.into(), is_dir: true }
    }
    fn file(name: &str) -> Entry {
        Entry { name: name.into(), is_dir: false }
    }

    #[test]
    fn extension_never_participates() {
        let (items, counts) = preview_level("L:/x", None, &[file("名称!试听.mp3")]);
        assert_eq!(items[0].target, "名称！试听.mp3", "扩展名原样拼回");
        assert_eq!(counts.changed, 1);
        // 名字里带的点不算扩展名，只有最后一个才是
        let (items, _) = preview_level("L:/x", None, &[file("01.名称~.mp3")]);
        assert_eq!(items[0].target, "01.名称～.mp3");
    }

    #[test]
    fn untouched_entries_are_not_listed_but_still_counted() {
        let (_, counts) = preview_level(
            "L:/x",
            None,
            &[dir("深夜掏耳"), dir("名称  空格"), dir("01_测试")],
        );
        assert_eq!(counts.scanned, 3);
        assert_eq!((counts.compliant, counts.normalize_only), (2, 1));
        let (items, _) = preview_level("L:/x", None, &[dir("名称  空格")]);
        assert_eq!((items.len(), items[0].target.as_str()), (1, "名称 空格"));
    }

    #[test]
    fn mixed_prefix_families_report_instead_of_rewriting() {
        let entries = [dir("01_名前的~测试"), dir("【02】別の~测试")];
        let (items, counts) = preview_level("L:/x", None, &entries);
        assert_eq!(items.len(), 2);
        for it in &items {
            assert_eq!(it.verdict, Verdict::NormalizeOnly);
            assert!(it.reasons.contains(&Reason::PrefixMixed));
            assert!(!it.needs_ai, "族不统一的不进模型（§7.2 第 8 条排在假名之前）");
        }
        assert_eq!(counts.normalize_only, 2);
        // 同一族不该报 prefix_mixed
        let same = [dir("01_名前的~测试"), dir("02_別の~测试")];
        let (items, counts) = preview_level("L:/x", None, &same);
        assert!(!items[0].reasons.contains(&Reason::PrefixMixed));
        assert_eq!(counts.normalize_only, 2);
    }

    #[test]
    fn same_target_and_taken_target_are_both_conflicts() {
        // 两条各自规范化后撞名：两边都不能自动改
        let (items, counts) = preview_level("L:/x", None, &[dir("名称~甲"), dir("名称〜甲")]);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.conflict == Some(Conflict::SameTarget)));
        assert_eq!(counts.conflicts, 2);
        // 目标名已被旁边一条占着（那条自己不改，所以不算撞）
        let taken = [dir("名称～甲"), dir("名称〜乙"), dir("名称～乙")];
        let (items, _) = preview_level("L:/x", None, &taken);
        assert_eq!(items.len(), 1, "只有中间那条有变化");
        assert_eq!(items[0].conflict, Some(Conflict::TargetExists));
    }

    #[test]
    fn over_long_full_path_is_a_conflict_not_a_silent_cut() {
        let deep = format!("L:/{}", "层".repeat(239));
        let (items, counts) = preview_level(&deep, None, &[dir("名  字")]);
        assert_eq!(items[0].conflict, Some(Conflict::PathTooLong));
        assert_eq!(counts.conflicts, 1);
        let (items, _) = preview_level("L:/浅", None, &[dir("名  字")]);
        assert_eq!(items[0].conflict, None, "路径不长就不该报");
    }

    #[test]
    fn duplicate_title_only_applies_to_files_inside_the_work_folder() {
        let (items, _) = preview_level("L:/深夜掏耳", Some("深夜掏耳"), &[file("深夜掏耳_第二夜~.flac")]);
        assert!(items[0].reasons.contains(&Reason::DuplicateTitle));
        assert_eq!(items[0].verdict, Verdict::NormalizeOnly);
        // 文件夹条目自己不可能是「重复了所属作品名」
        let (_, counts) = preview_level("L:/x", Some("深夜掏耳"), &[dir("深夜掏耳")]);
        assert_eq!((counts.normalize_only, counts.compliant), (0, 1));
    }

    #[test]
    fn needs_ai_marks_what_still_has_to_go_to_the_model() {
        let (items, counts) = preview_level("L:/x", None, &[dir("精霊の魔力"), dir("深夜掏耳")]);
        assert_eq!(items.len(), 1, "已合规又不必动的不列出");
        assert_eq!(items[0].verdict, Verdict::NeedsTranslation);
        assert!(items[0].needs_ai);
        assert!(!items[0].changed, "本地规则一条都没改着，但它仍要进模型");
        assert_eq!((counts.scanned, counts.compliant), (2, 1));
        // 符号＋假名：原名第一命中是 wave（本地档），可收掉波浪号之后假名还在，照样要模型
        let (items, _) = preview_level("L:/x", None, &[dir("精霊の魔力~")]);
        assert_eq!(items[0].verdict, Verdict::NormalizeOnly);
        assert!(items[0].needs_ai, "规范化之后重判，不然这条永久停在本地档");
        // 反过来：痕迹只是尾部残留分隔符的，规则修完就不必花 token
        let (items, _) = preview_level("L:/x", None, &[dir("掏耳_")]);
        assert_eq!(items[0].verdict, Verdict::NeedsRepair);
        assert_eq!(items[0].target, "掏耳");
        assert!(!items[0].needs_ai);
    }

    #[test]
    fn preview_level_keeps_the_order_it_was_given() {
        // 排序是列目录那一步的事（listing.rs）；这里保持入参顺序，两层职责别缠在一起
        let (items, _) = preview_level("L:/x", None, &[file("b~.mp3"), dir("a~")]);
        assert_eq!(
            items.iter().map(|i| i.kind).collect::<Vec<_>>(),
            vec![Kind::File, Kind::Folder]
        );
    }
}
