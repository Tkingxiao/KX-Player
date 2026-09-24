//! 改名那一侧的**意图外账**（P1-11）：动手之前写下「打算把哪些名字搬成什么」，收尾时删掉。
//!
//! 三条崩溃窗口，形状都是「盘动了、账没动」：
//! 1. `move_one` 成功之后、`add_item` 之前 —— 文件已经在新名下，库里一个字没记，回滚时找不到它；
//! 2. 一批改到一半 —— 前几条有账，批次头还挂着 `applied_at = NULL`，`finish` 再也没跑；
//! 3. 回滚/重做改到一半 —— 同一件事，方向反过来（旗子翻了、下一半没翻）。
//!
//! 恢复的立场只有一条：**盘是事实，恢复只补账**。启动时绝不替用户继续搬文件 —— 库里那本账
//! 记的是「预览那一刻的绝对路径」，而从崩溃到下次启动之间用户完全可能自己改过名、删过文件；
//! 这时候照着 intent 硬搬才是真的丢数据。所以逐条按盘上的现状判定：
//! - 目标名在、原名字不在 → 这条生效了 → 把库里的行对齐（缺了就补，旗子反了就翻回来）；
//! - 原名字在、目标名不在 → 这条没生效（当年本来就失败，或者崩在它之前）→ 不动，计数；
//! - 两个都在 / 两个都不在 → 说不清，报出来，**盘和账都不动**（宁可让人来看一眼）。
//!
//! 最后补批次收尾与 `rename_history.json` —— 两处都是幂等的（`merge` 值相同不算改动、
//! `revert` 只撤「值还是本批写的那个」），所以恢复跑两遍和跑一遍一样。

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::journal;
use crate::rename::batch::{self, Pair, FILE, FOLDER};
use crate::rename::paths::resolve;
use crate::rename::report::{sync, Report};

/// 意图文件的统一前缀（`journal::list` 按它筛出「改名的」那几份）。
pub const PREFIX: &str = "rename-";
/// `Intent.dir` 的三个取值。回滚与重做共用 `reverse()`，靠这个字段分出方向。
pub const APPLY: &str = "apply";
pub const ROLLBACK: &str = "rollback";
pub const REDO: &str = "redo";

const RECOVERED: &str = "（崩溃后恢复）";

/// 一篇意图。`pairs` 是**执行顺序**（`paths::sort_pairs` 排完的那一份），
/// 恢复要按同一个顺序重放，才能把「父目录先改了名」这类换算还原出来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    pub batch_id: String,
    /// [`APPLY`] / [`ROLLBACK`] / [`REDO`]
    pub dir: String,
    pub started_at: i64,
    pub pairs: Vec<Entry>,
    /// history 那一侧要用的根。应用时是用户勾的那几个目录，反向是各条的父目录 ——
    /// 不是「成功的那几条的父目录」：恢复时手里只有这一份，两边必须同源。
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub old: String,
    pub new: String,
    pub level: String,
}

impl Intent {
    pub fn new(batch_id: &str, dir: &str, pairs: &[Pair], roots: &[String]) -> Self {
        Self {
            batch_id: batch_id.to_string(),
            dir: dir.to_string(),
            started_at: now_ms(),
            pairs: pairs
                .iter()
                .map(|p| Entry { old: p.old.clone(), new: p.new.clone(), level: p.level.to_string() })
                .collect(),
            roots: roots.to_vec(),
        }
    }

    fn entry_pair(e: &Entry) -> Pair {
        Pair {
            old: e.old.clone(),
            new: e.new.clone(),
            level: if e.level == FOLDER { FOLDER } else { FILE },
        }
    }
}

pub(crate) fn filename(batch_id: &str) -> String {
    format!("{PREFIX}{batch_id}.json")
}

/// 动手之前写下这一批要做什么。返回 `Err` 时调用方必须**一个字节都不动** ——
/// 写不下这份文件的批次，崩溃后没有任何线索，不值得开工。
pub fn write(intent: &Intent) -> Result<(), String> {
    let body = serde_json::to_string_pretty(intent).map_err(|e| format!("序列化失败：{e}"))?;
    journal::put(&journal::dir(), &filename(&intent.batch_id), &body)
}

/// 收尾：盘、库、history 三处都对齐了才收掉这份意图。
pub fn clear(batch_id: &str) {
    journal::remove(&journal::dir(), &filename(batch_id));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// 一份被核对过的意图。`failed` 里是「说不清」的条目明细（两端都在/都不在这类）。
#[derive(Debug, Clone)]
pub struct Recovered {
    pub batch_id: String,
    pub dir: String,
    pub report: Report,
}

impl Recovered {
    /// 日志里那一行（带明细，路径也在里头 —— 日志本来就是给排查用的）。
    pub fn line(&self) -> String {
        let mut s = format!(
            "批次 {} · {} · 计划 {} · 已生效 {} · 未生效 {} · 说不清 {}",
            self.batch_id,
            self.dir,
            self.report.planned,
            self.report.applied,
            self.report.skipped,
            self.report.failed.len()
        );
        if self.report.history_written > 0 {
            s.push_str(&format!(" · history 补记 {} 条", self.report.history_written));
        }
        if !self.report.failed.is_empty() {
            s.push_str(&format!(" · 明细：{}", self.report.failed.join(" | ")));
        }
        s
    }

    /// 启动提示里那一句（给人看的，只说条数与「去看哪儿」）。
    pub fn warn(&self) -> String {
        let mut s = format!(
            "上次改名没有正常结束（批次 {}，{}）：已按盘上的现状补齐账目 —— 生效 {} 条 · 未生效 {} 条",
            self.batch_id, self.dir, self.report.applied, self.report.skipped
        );
        if !self.report.failed.is_empty() {
            s.push_str(&format!(" · 有 {} 条对不上（明细见日志）", self.report.failed.len()));
        }
        s.push_str("。请到改名页复核后再继续。");
        s
    }
}

/// 启动恢复入口（`lib.rs` setup 调用）：结果三条路 —— 日志一行（总能查）、启动提示一条
/// （用户能看见）、以及曲库打不开时**保留意图**（什么都没核对，下次再试）。
pub fn recover_at_startup() {
    let dir = journal::dir();
    let files = journal::list(&dir, PREFIX);
    if files.is_empty() {
        return;
    }
    let db = crate::paths::library_db_path();
    if !db.exists() {
        // 曲库还没建起来（重装/清库之后）：没有可对齐的账，意图文件留着等曲库回来
        crate::paths::append_log(&format!(
            "[重命名] {} 份未完成的意图还在，但曲库不存在，未核对",
            files.len()
        ));
        return;
    }
    match crate::db::with_db_text(&db, |conn| Ok(recover_all(conn, &dir))) {
        Ok(list) => {
            for r in &list {
                crate::paths::append_log(&format!("[重命名] 崩溃后恢复：{}", r.line()));
                crate::state::startup_warn(r.warn());
            }
        }
        Err(e) => {
            crate::paths::append_log(&format!("[重命名] 崩溃后恢复未执行：{e}"));
            crate::state::startup_warn(
                "上次改名似乎没有正常结束，但这次没能核对账目（曲库打不开）。未做任何改动，可在改名页复核。",
            );
        }
    }
}

/// 扫一遍目录里的改名意图，逐份核对。**只有核对完的才收掉那份文件**：核对不了的
/// （曲库里没有这一批、文件读不出来）留在原地，下次启动再试，日志与提示里如实说。
pub fn recover_all(conn: &Connection, dir: &Path) -> Vec<Recovered> {
    let mut out = Vec::new();
    for path in journal::list(dir, PREFIX) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let intent = match journal::read(&path).and_then(|text| {
            serde_json::from_str::<Intent>(&text).map_err(|e| format!("解析失败：{e}"))
        }) {
            Ok(v) => v,
            Err(e) => {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
                out.push(Recovered {
                    batch_id: stem,
                    dir: "?".into(),
                    report: Report::with_failure(0, format!("意图文件读不出来（{e}），未做任何改动")),
                });
                continue;
            }
        };
        match reconcile(conn, &intent) {
            Ok(report) => {
                let rec = Recovered { batch_id: intent.batch_id.clone(), dir: intent.dir.clone(), report };
                journal::remove(dir, &filename(&intent.batch_id));
                out.push(rec);
            }
            Err(e) => out.push(Recovered {
                batch_id: intent.batch_id.clone(),
                dir: intent.dir.clone(),
                report: Report::with_failure(intent.pairs.len(), format!("{e}，未做任何改动")),
            }),
        }
    }
    out
}

/// 把一份意图按**盘上的现状**对齐到库里。`Err` = 这一次没能核对（保留意图文件，下次再试）。
///
/// 不搬文件、不覆盖 —— 只写 `rename_items` 的旗子、批次头与 history。`moved` 的重放和运行时
/// 那台机器同一个道理：父目录改过名之后，子项存的两端都得按已改的那些换算成现名。
fn reconcile(conn: &Connection, intent: &Intent) -> Result<Report, String> {
    if !batch::exists(conn, &intent.batch_id)? {
        return Err(format!("曲库里找不到批次 {}", intent.batch_id));
    }
    // 方向：applied=1 = 盘上该是新名
    let forward = intent.dir != ROLLBACK;
    let flag = i64::from(forward);
    let mut rep = Report::new(intent.batch_id.clone(), intent.pairs.len(), 0);
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut done: Vec<Pair> = Vec::new();
    for e in &intent.pairs {
        let p = Intent::entry_pair(e);
        let (cur, want) = if forward { (&p.old, &p.new) } else { (&p.new, &p.old) };
        let from = resolve(cur, &moved);
        let to = resolve(want, &moved);
        let (src, dst) = (Path::new(&from).exists(), Path::new(&to).exists());
        let state = batch::item_state(conn, &intent.batch_id, &p.old)?;
        match (src, dst) {
            // 生效了：把账对齐。缺行 = 崩在 add_item 之前；旗子反了 = 崩在 set_item 之前
            (false, true) => {
                match state {
                    None => batch::add_item(conn, &intent.batch_id, &p, forward)?,
                    Some(a) if a == flag => {}
                    Some(_) => batch::set_item(conn, &intent.batch_id, &p, forward)?,
                }
                rep.applied += 1;
                moved.push((cur.clone(), want.clone()));
                done.push(p);
            }
            // 没生效。库里却有一行说它生效了 —— 那是崩溃之后盘又被谁动过，报出来，两边都不碰
            (true, false) => {
                if state == Some(flag) {
                    let what = if forward { "改名" } else { "回滚" };
                    rep.failed.push(format!("{from}：账上记着这一条已{what}，盘上却还叫这个名字"));
                } else {
                    rep.skipped += 1;
                }
            }
            (true, true) => rep.failed.push(format!("{from} 与 {to} 同时存在，未改动")),
            (false, false) => rep.failed.push(format!("{from} 与 {to} 都不存在，未改动")),
        }
    }

    // 批次头：`applied_at` 还空着 = 那一次应用根本没跑完，补上；否则只翻 `rolled_back` 与说明 ——
    // `applied_at` 记的是**首次应用**，恢复不该把它改写成「现在」。
    let label = match intent.dir.as_str() {
        ROLLBACK => "回滚",
        REDO => "重做",
        _ => "应用",
    };
    let (planned, applied, skipped, failed) = (rep.planned, rep.applied, rep.skipped, rep.failed.len());
    if batch::applied_at(conn, &intent.batch_id)?.is_none() {
        batch::finish(conn, &intent.batch_id, &format!("{label}{RECOVERED}"), planned, applied, skipped, failed)?;
    } else {
        let note = format!("{label}{RECOVERED} · 计划 {planned} · 成功 {applied} · 跳过 {skipped} · 失败 {failed}");
        batch::mark_rolled_back(conn, &intent.batch_id, intent.dir == ROLLBACK, &note)?;
    }
    // history 那一侧：运行时的 `sync` 崩在哪儿就在哪儿补（幂等，见模块头）
    let undo = intent.dir == ROLLBACK;
    sync(&mut rep, &intent.roots, &done, undo);
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::{filename, recover_all, Intent, APPLY, PREFIX, REDO, ROLLBACK};
    use crate::journal;
    use crate::rename::batch::{self, Pair, FILE};
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    /// 表由**真迁移**建，不是手抄的 DDL（与 `apply.rs` 的夹具同口径）
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::migrations::apply(&conn).expect("建表");
        conn
    }

    /// 临时树 + 一个独立的意图目录。意图目录放在树里但单独一层，断言「盘上有什么」时不会混进来。
    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("kx-rj-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("nor")).expect("根目录");
            std::fs::create_dir_all(root.join("journal")).expect("意图目录");
            Self(root)
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
        fn jdir(&self) -> PathBuf {
            self.0.join("journal")
        }
        fn names(&self, rel: &str) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(self.path(rel))
                .expect("列目录")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    fn pair(old: &Path, new: &Path) -> Pair {
        Pair { old: s(old), new: s(new), level: FILE }
    }

    /// 崩溃现场要**手工摆出来**：把「盘上已经是什么样、库里记了几行、意图还在不在」照那一刻摆好，
    /// 再让恢复去跑。进程内没法在 `move_one` 与 `add_item` 之间真的被杀掉，但那一刻的状态是可复现的。
    fn write_intent(dir: &Path, intent: &Intent) {
        let body = serde_json::to_string(intent).expect("序列化");
        crate::atomic::write_str(&dir.join(filename(&intent.batch_id)), &body).expect("摆意图文件");
    }

    fn row(conn: &Connection, id: &str) -> (Option<i64>, i64, i64, Option<String>) {
        conn.query_row(
            "SELECT applied_at, item_count, rolled_back, note FROM rename_batches WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("批次行")
    }

    fn history_of(t: &Tree) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(t.path("rename_history.json")).unwrap()).unwrap()
    }

    /// 崩溃窗口 1（`move_one` 成了、`add_item` 还没写）：盘上文件已经在新名下，库里一行都没有。
    /// 恢复要补这一行、收尾批次、补 history，然后收掉意图文件。
    #[test]
    fn a_moved_but_unrecorded_item_is_booked_at_the_next_start() {
        let t = Tree::new("booked");
        std::fs::write(t.path("nor").join("特典～"), "x").unwrap();
        std::fs::write(t.path("rename_history.json"), "{}").unwrap();

        let conn = db();
        batch::open(&conn, "rbcrash1", None).unwrap();
        let p = pair(&t.path("nor").join("特典~"), &t.path("nor").join("特典～"));
        write_intent(&t.jdir(), &Intent::new("rbcrash1", APPLY, std::slice::from_ref(&p), &[s(&t.path("nor"))]));

        let out = recover_all(&conn, &t.jdir());
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].report.planned, out[0].report.applied, out[0].report.failed.len()), (1, 1, 0));
        assert_eq!(batch::pairs(&conn, "rbcrash1", 1).unwrap(), vec![p.clone()], "缺的那行要补回来");
        assert!(batch::applied_at(&conn, "rbcrash1").unwrap().is_some(), "批次要收尾");
        assert_eq!(row(&conn, "rbcrash1").1, 1, "item_count 按真实条数记");
        assert_eq!(t.names("nor"), ["特典～"], "恢复不动盘");
        assert!(journal::list(&t.jdir(), PREFIX).is_empty(), "核对完就收掉意图");
        // 组键：文件直接躺在根下、根自己就是作品目录 —— 按 `04 §7.1` 第 4 条退回用根名开组
        assert_eq!(history_of(&t)["nor"]["nor"]["特典~"], "特典～", "history 也要补上");
    }

    /// 崩溃窗口 2（一批改到一半）：前一条有账、后一条一个字没动，批次头还挂着 `applied_at = NULL`。
    /// 恢复只把账对齐，**不替用户把后一条改完** —— 后一条算「没生效」，批次按现状收尾。
    #[test]
    fn a_half_done_batch_is_closed_with_the_disk_as_the_truth() {
        let t = Tree::new("half");
        std::fs::write(t.path("nor").join("a.mp3"), "x").unwrap(); // 已生效
        std::fs::write(t.path("nor").join("b~.mp3"), "y").unwrap(); // 没动

        let conn = db();
        batch::open(&conn, "rbcrash2", None).unwrap();
        let p1 = pair(&t.path("nor").join("a~.mp3"), &t.path("nor").join("a.mp3"));
        let p2 = pair(&t.path("nor").join("b~.mp3"), &t.path("nor").join("b.mp3"));
        batch::add_item(&conn, "rbcrash2", &p1, true).unwrap();
        write_intent(&t.jdir(), &Intent::new("rbcrash2", APPLY, &[p1.clone(), p2], &[s(&t.path("nor"))]));

        let out = recover_all(&conn, &t.jdir());
        assert_eq!((out[0].report.applied, out[0].report.skipped, out[0].report.failed.len()), (1, 1, 0));
        assert_eq!(batch::pairs(&conn, "rbcrash2", 1).unwrap(), vec![p1], "没生效的那条不该进账");
        assert_eq!(t.names("nor"), ["a.mp3", "b~.mp3"], "不替用户继续搬文件");
        let (applied_at, item_count, rolled_back, note) = row(&conn, "rbcrash2");
        assert!(applied_at.is_some(), "批次要收尾，不然它永远停在「没跑完」");
        assert_eq!((item_count, rolled_back), (1, 0));
        assert!(note.unwrap().starts_with("应用（崩溃后恢复）"), "说明里要看出这是恢复出来的");
    }

    /// 「两端都在」说不清：**盘和账都不动**，只把话说明白（宁可让人来看一眼）。
    #[test]
    fn an_ambiguous_item_is_reported_and_left_alone() {
        let t = Tree::new("ambiguous");
        std::fs::write(t.path("nor").join("甲~"), "旧").unwrap();
        std::fs::write(t.path("nor").join("甲"), "新").unwrap();

        let conn = db();
        batch::open(&conn, "rbcrash3", None).unwrap();
        let p = pair(&t.path("nor").join("甲~"), &t.path("nor").join("甲"));
        write_intent(&t.jdir(), &Intent::new("rbcrash3", APPLY, &[p], &[s(&t.path("nor"))]));

        let out = recover_all(&conn, &t.jdir());
        assert_eq!((out[0].report.applied, out[0].report.failed.len()), (0, 1));
        assert!(out[0].report.failed[0].contains("同时存在"), "{:?}", out[0].report.failed);
        assert!(batch::pairs(&conn, "rbcrash3", 1).unwrap().is_empty(), "说不清的条目不进账");
        assert_eq!(std::fs::read_to_string(t.path("nor").join("甲~")).unwrap(), "旧");
        assert_eq!(std::fs::read_to_string(t.path("nor").join("甲")).unwrap(), "新");
        assert!(batch::applied_at(&conn, "rbcrash3").unwrap().is_some(), "批次照样收尾，失败数记在说明里");
    }

    /// 回滚崩在一半：文件已经搬回原名、旗子还没翻。恢复按**方向**把旗子翻成 0、批次标成已回滚，
    /// 而 `applied_at` 一个字都不能动 —— 它记的是**首次应用**。
    #[test]
    fn a_crashed_rollback_flips_the_flags_but_not_applied_at() {
        let t = Tree::new("rollback");
        std::fs::write(t.path("nor").join("特典~"), "x").unwrap();

        let conn = db();
        batch::open(&conn, "rbcrash4", None).unwrap();
        let p = pair(&t.path("nor").join("特典~"), &t.path("nor").join("特典～"));
        batch::add_item(&conn, "rbcrash4", &p, true).unwrap();
        batch::finish(&conn, "rbcrash4", "本地规则", 1, 1, 0, 0).unwrap();
        let before = batch::applied_at(&conn, "rbcrash4").unwrap().unwrap();

        write_intent(&t.jdir(), &Intent::new("rbcrash4", ROLLBACK, std::slice::from_ref(&p), &[s(&t.path("nor"))]));
        let out = recover_all(&conn, &t.jdir());
        assert_eq!(out[0].report.applied, 1);
        assert!(batch::pairs(&conn, "rbcrash4", 1).unwrap().is_empty(), "旗子翻回 0 = 这条已回滚");
        assert_eq!(batch::pairs(&conn, "rbcrash4", 0).unwrap(), vec![p]);
        let (applied_at, _, rolled_back, note) = row(&conn, "rbcrash4");
        assert_eq!(rolled_back, 1);
        assert_eq!(applied_at, Some(before), "applied_at 不该被改写成「现在」");
        assert!(note.unwrap().starts_with("回滚（崩溃后恢复）"));
    }

    /// 恢复是幂等的：再跑一遍不会多出一行账、也不会往 history 里写第二遍
    /// （`merge` 认「值没变就不算改动」）。这是「恢复到一半又崩」还敢再来一次的前提。
    #[test]
    fn running_recovery_twice_changes_nothing_the_second_time() {
        let t = Tree::new("twice");
        std::fs::write(t.path("nor").join("歌 手.srt"), "x").unwrap();
        std::fs::write(t.path("rename_history.json"), "{}").unwrap();

        let conn = db();
        batch::open(&conn, "rbcrash5", None).unwrap();
        let p = pair(&t.path("nor").join("歌  手.srt"), &t.path("nor").join("歌 手.srt"));
        let intent = Intent::new("rbcrash5", APPLY, std::slice::from_ref(&p), &[s(&t.path("nor"))]);

        write_intent(&t.jdir(), &intent);
        let first = recover_all(&conn, &t.jdir());
        assert_eq!((first[0].report.applied, first[0].report.history_written), (1, 1));
        let doc = history_of(&t);

        // 恢复到一半又崩：意图文件还在（这里手工再摆一份），第二次跑不能有任何新增
        write_intent(&t.jdir(), &intent);
        let again = recover_all(&conn, &t.jdir());
        assert_eq!(again[0].report.applied, 1, "盘上确实是新名，这条仍算已生效");
        assert_eq!(again[0].report.history_written, 0, "history 里已经是对的，不该再写一遍");
        assert_eq!(batch::pairs(&conn, "rbcrash5", 1).unwrap(), vec![p], "不许出现第二行");
        assert_eq!(row(&conn, "rbcrash5").1, 1, "item_count 也不该翻倍");
        assert_eq!(history_of(&t), doc);
        // 重做方向的那份意图也只是核对，不该把文件搬回去
        write_intent(&t.jdir(), &Intent::new("rbcrash5", REDO, &[pair(&t.path("nor").join("歌  手.srt"), &t.path("nor").join("歌 手.srt"))], &[s(&t.path("nor"))]));
        let redo = recover_all(&conn, &t.jdir());
        assert_eq!(redo[0].report.applied, 1, "重做方向：盘上已经是新名，账也对得上");
        assert_eq!(t.names("nor"), ["歌 手.srt"], "恢复永远不动盘");
    }

    /// 曲库里没有这一批（库被重建/清空过）：核对不了就**留着意图文件**，下次启动再试 ——
    /// 这份文件是「当时到底打算搬什么」的唯一记录，不能因为一次对不上就丢掉。
    #[test]
    fn an_unmatchable_intent_is_kept_for_the_next_start() {
        let t = Tree::new("unknown");
        let conn = db();
        let p = pair(&t.path("nor").join("甲~"), &t.path("nor").join("甲"));
        write_intent(&t.jdir(), &Intent::new("rb没影了", APPLY, &[p], &[s(&t.path("nor"))]));

        let out = recover_all(&conn, &t.jdir());
        assert_eq!(out.len(), 1);
        assert!(out[0].report.failed[0].contains("曲库里找不到批次"), "{:?}", out[0].report.failed);
        assert_eq!(journal::list(&t.jdir(), PREFIX).len(), 1, "留证据，下次再试");
        assert!(out[0].warn().contains("rb没影了"), "提示里要说清是哪一批");
    }
}
