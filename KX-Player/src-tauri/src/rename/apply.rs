//! **正向执行**（`04 §7.7` 会写盘的那一半）：把一批已经定稿的改名落到盘上。
//!
//! 三条不能弯的规矩：
//! 1. **执行的就是预览的那份判定** —— 收的是 `roots`，内部原样再调一次 `listing::preview_roots`，
//!    不接受前端回传条目。预览与落地之间隔着一双手（用户可能删过文件、换过选择），而
//!    「列表一变、手上那份映射就指向别的文件」是第 17 批记下的头号数据风险（接力棒坑 33）。
//!    重算一次的代价是几次 `read_dir`，换来的是「按钮改的 == 屏幕上写的」。
//! 2. **只动定稿的条目**：本地规则改完就算定稿；要问模型的那条**必须过译名缓存** ——
//!    `ai_ledger::record` 只把过了九道校验的名字写进那张表，所以查得到就是「已核准」，
//!    查不到继续算 `skipped`。前端回传的名字从来没有一条路能进这里（规矩 1 的另一半）。
//! 3. **改完一项立刻记一项**（`04 §7.7` 第 3 条）。记账失败就当项撤销 —— 回滚全靠这份账，
//!    一条没落库的成功改名等于一个再也回不去的文件。
//!
//! 反方向的两个命令（回滚 / 重做）在 `rollback.rs`，两者共用的是 `report.rs`（汇总形状、当项撤销、
//! history 记账）和 `paths.rs` 那套顺序与换算，**整个 `rename/` 里只有这两个文件碰文件系统**，
//! 其余都是纯函数或只 `read_dir`。

use std::collections::HashSet;
use std::path::Path;

use rusqlite::Connection;

use crate::rename::ai_ledger;
use crate::rename::batch::{self, Pair, FOLDER};
use crate::rename::paths::{move_one, resolve, sort_pairs};
use crate::rename::preview::{Item, Kind, Preview};
use crate::rename::report::{sync, undo, Report};

/// 应用：重算预览 → 只挑定稿的条目（本地规则 + 译名缓存里已核准的）→ 按 `04 §7.7` 的顺序改盘 →
/// 逐项落库 → 追加 history。
pub fn apply(conn: &Connection, roots: &[String]) -> Result<Report, String> {
    let preview = crate::rename::listing::preview_roots(roots);
    if preview.truncated {
        // 截断的是**列表**不是计数：照单执行会静默漏掉上限以外的条目，那比报错坏得多
        return Err(format!(
            "本批候选超过 {} 条预览上限，请减少选择的目录后分批应用",
            crate::rename::listing::MAX_ITEMS
        ));
    }
    let Queue { pairs, skipped, model, ai } = queue(conn, &preview);
    let id = batch::new_batch_id();
    batch::open(conn, &id, model.as_deref())?;

    let mut rep = Report::new(id.clone(), pairs.len(), skipped);
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut done: Vec<Pair> = Vec::new();
    for p in &pairs {
        // 父目录可能已被本批改过名（所选根相互嵌套时会有），先把这条的**两端**都换算成现在的真实路径
        let from = resolve(&p.old, &moved);
        let to = resolve(&p.new, &moved);
        if let Err(e) = move_one(&from, &to) {
            rep.failed.push(format!("{from}：{e}"));
            continue;
        }
        match batch::add_item(conn, &id, p) {
            Ok(()) => {
                rep.applied += 1;
                moved.push((p.old.clone(), p.new.clone()));
                done.push(p.clone());
            }
            // 账没记上就撤销这次改名：宁可回到改之前，也不留一个再也回不去的文件
            Err(e) => {
                rep.failed.push(format!("{}：{e}，已撤销", p.old));
                undo(&mut rep, &to, &from);
            }
        }
    }
    let (planned, applied, skipped, failed) = (rep.planned, rep.applied, rep.skipped, rep.failed.len());
    // 说明里带上这批有没有模型参与：`04 §7.11` 要能一眼看出本地规则批次一分钱 token 没花
    let kind = if ai > 0 { "本地规则 + 模型译名" } else { "本地规则" };
    batch::finish(conn, &id, kind, planned, applied, skipped, failed)?;
    sync(&mut rep, roots, &done, false);
    Ok(rep)
}

/// 待执行队列 + 三件顺带算出来的事：跳过几条、这批用过哪个模型、有几条来自模型。
struct Queue {
    pairs: Vec<Pair>,
    skipped: usize,
    model: Option<String>,
    ai: usize,
}

/// 预览结果 → 待执行队列。两条来路：本地规则已经改出名字的，直接用 `target`；
/// 要问模型的，只认**译名缓存**里那条已核准的名字（`ai_ask` 只把过完九道校验的写进去），
/// 查不到就还是 `skipped` —— 宁可什么都不动，也不会把一个没校验过的名字写上盘。
fn queue(conn: &Connection, p: &Preview) -> Queue {
    let mut pairs = Vec::new();
    let mut skipped = 0usize;
    let mut model: Option<String> = None;
    let mut ai = 0usize;
    // 同一目录里目标名的占用集：本地目标预览已经查过冲突，模型译名是新算出来的，
    // 两个不同原文翻成同一个词是常态 —— 在这里挡住，比到 `move_one` 那一步报错干净。
    let mut taken: HashSet<(String, String)> = HashSet::new();
    for it in &p.items {
        // 先问「该不该动」再问「有没有变化」：没定稿的那条即使本地一撇都没改，也是
        // 这一批**没替用户做**的事，得计入 skipped 让人看见
        let Some(target) = target_for(conn, it, &mut model) else {
            if it.needs_ai || it.conflict.is_some() {
                skipped += 1;
            }
            continue;
        };
        // 名字没变就不动。判据是**最终目标名**而不是 `changed`：`changed` 说的只是本地规则，
        // 而模型译名常常落在「本地一撇没改、只有翻译才改得动」的那条上。
        if target == it.name {
            continue;
        }
        let Some(dir) = Path::new(&it.path).parent() else {
            skipped += 1;
            continue;
        };
        let dir = dir.to_string_lossy().into_owned();
        if it.conflict.is_some() || !taken.insert((dir.clone(), target.clone())) {
            skipped += 1;
            continue;
        }
        if it.needs_ai {
            ai += 1;
        }
        pairs.push(Pair {
            old: it.path.clone(),
            new: Path::new(&dir).join(&target).to_string_lossy().into_owned(),
            level: match it.kind {
                Kind::Folder => FOLDER,
                Kind::File => batch::FILE,
            },
        });
    }
    sort_pairs(&mut pairs, false);
    Queue { pairs, skipped, model, ai }
}

/// 这一条最终要改成什么，`None` = 不由这一批来动。顺带记下用过的模型名（审计字段，`04 §7.8`）。
fn target_for(conn: &Connection, it: &Item, model: &mut Option<String>) -> Option<String> {
    if it.conflict.is_some() {
        return None;
    }
    if !it.needs_ai {
        return Some(it.target.clone());
    }
    let (name, used) = ai_ledger::suggestion(conn, it)?;
    if *model != used {
        // 一批里换了模型是事实，不是错误：记最后一个，明细里每一行本来就带着自己的模型名
        *model = used;
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    // 回滚与重做的用例也写在这一格：它们断言的是「应用 ↔ 回滚」**这一对**合起来成立，单独测
    // `rollback.rs` 测不出配对；建库、建临时目录那几个夹具也因此只用写一份。
    use super::apply;
    use crate::rename::ai_ask::Outcome;
    use crate::rename::ai_ledger::record;
    use crate::rename::ai_name;
    use crate::rename::preview::{preview_level, Entry, Item};
    use crate::rename::rollback::{redo, rollback};
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    /// 表由**真迁移**建，不是手抄的 DDL —— 忘了往 `MIGRATIONS` 里加那一行，这里就该红
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::migrations::apply(&conn).expect("建表");
        conn
    }

    fn tree(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("kx-apply-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("临时目录");
        p
    }

    fn root_of(base: &Path, name: &str) -> String {
        let r = base.join(name);
        std::fs::create_dir_all(&r).expect("根目录");
        r.to_string_lossy().into_owned()
    }

    /// 一层目录的名字列表（排序后比对，文件系统不保证顺序）
    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .expect("列目录")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// 期望值也排序：断言里写的是「有哪些」，不该逼作者记住 CJK 的字节序
    fn assert_names(dir: &Path, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(names(dir), want, "目录 {dir:?} 里的名字");
    }

    /// 三种 separator 混用的显示串，比对前统一成 `/`
    fn sl(p: &str) -> String {
        p.replace('\\', "/")
    }

    fn count_items(conn: &Connection, id: &str) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM rename_items WHERE batch_id = ?1", [id], |r| r.get(0))
            .expect("条目数")
    }

    fn batch_row(conn: &Connection, id: &str) -> (Option<i64>, i64, i64, Option<String>) {
        conn.query_row(
            "SELECT applied_at, item_count, rolled_back, note FROM rename_batches WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("批次行")
    }

    fn model_of(conn: &Connection, id: &str) -> Option<String> {
        conn.query_row("SELECT model FROM rename_batches WHERE id = ?1", [id], |r| r.get(0)).expect("批次行")
    }

    /// 一条「已过校验」的译名。片段与键都用**落地侧同一套算法**算，用例里不另抄一份口径 ——
    /// 两边算出不同的键，这个用例就会红，而那正是「查得到却用不上」的成因。
    fn approved(it: &Item, dst: &str) -> Outcome {
        let src = ai_name::fragment(it);
        Outcome { key: ai_name::cache_key(&src), src, dst: Some(dst.into()), notes: vec![], blocked: vec![] }
    }

    /// 三个名字来自 `listing.rs` 的既有用例：`特典~` 与 `歌  手.srt` 该被本地规则改掉，
    /// `正常名字.lrc` 一条都不动。
    #[test]
    fn apply_renames_local_rules_and_books_every_item() {
        let base = tree("apply");
        let nor = root_of(&base, "nor");
        std::fs::create_dir(base.join("nor").join("特典~")).unwrap();
        std::fs::write(base.join("nor").join("歌  手.srt"), "x").unwrap();
        std::fs::write(base.join("nor").join("正常名字.lrc"), "x").unwrap();

        let conn = db();
        let rep = apply(&conn, std::slice::from_ref(&nor)).expect("应用");
        assert_eq!((rep.planned, rep.applied, rep.skipped, rep.failed.len()), (2, 2, 0, 0));
        assert_eq!(rep.history_file, None, "那一层没有 rename_history.json 就不该造一个");
        assert_names(&base.join("nor"), &["特典～", "正常名字.lrc", "歌 手.srt"]);
        // 注：断言不写死顺序（文件系统不保证），但「改该改的、不碰不该碰的」就是这三条

        let n = count_items(&conn, &rep.batch_id);
        assert_eq!(n, 2);
        let (applied_at, item_count, rolled_back, note) = batch_row(&conn, &rep.batch_id);
        assert!(applied_at.is_some(), "跑完了才有 applied_at");
        assert_eq!((item_count, rolled_back), (2, 0));
        assert_eq!(note.unwrap(), "本地规则 · 计划 2 · 成功 2 · 跳过 0 · 失败 0");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 所选根相互嵌套时（勾了作品根、又勾了它下面一层）：父目录已经改名之后，
    /// 子项的**两端**都得换算成现在的真实路径，不然就是「往一个不存在的目录里改名」。
    #[test]
    fn nested_roots_rename_the_parent_before_the_child() {
        let base = tree("nested");
        let outer = root_of(&base, "nor");
        let inner = root_of(&base.join("nor"), "特典~");
        std::fs::write(Path::new(&inner).join("歌  手.srt"), "x").unwrap();

        let conn = db();
        let rep = apply(&conn, &[outer.clone(), inner.clone()]).expect("应用");
        assert_eq!((rep.planned, rep.applied, rep.failed.len()), (2, 2, 0), "{:?}", rep.failed);
        assert_names(&base.join("nor"), &["特典～"]);
        // 内层根自己也是外层的一条，它一起改了名 —— 子项因此住在**新**目录下，而不是勾选时那一条
        assert_names(&Path::new(&outer).join("特典～"), &["歌 手.srt"]);
        // 库里记的仍是**预览时**的绝对路径（回滚以它为键），换算只发生在动手的那一刻
        let olds: Vec<String> = conn
            .prepare("SELECT old_path FROM rename_items WHERE batch_id = ?1")
            .expect("预编译")
            .query_map(rusqlite::params![rep.batch_id], |r| r.get::<_, String>(0))
            .expect("查询")
            .collect::<Result<Vec<_>, _>>()
            .expect("读行")
            .iter()
            .map(|o| sl(o))
            .collect();
        assert_eq!(olds.len(), 2);
        assert!(olds.iter().any(|o| o.ends_with("/nor/特典~")), "{olds:?}");
        assert!(olds.iter().any(|o| o.ends_with("/nor/特典~/歌  手.srt")), "{olds:?}");

        // 回滚的排序是自顶向下的：父目录先搬回原名，之后每一条存的 `new_path` 就是它当时的现名，
        // 所以一条都不会落到「现名已不存在」。这条断言就是那个顺序的存在理由。
        let back = rollback(&conn, &rep.batch_id).expect("回滚");
        assert_eq!((back.applied, back.failed.len()), (2, 0), "{:?}", back.failed);
        assert_names(&base.join("nor"), &["特典~"]);
        assert_names(Path::new(&inner), &["歌  手.srt"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 模型译名的**唯一**来路是那张「只写过校验的」库表：库里没有这一行，这一条就还是 `skipped`，
    /// 盘上一个字节都不动（`04 §7.7`）；补一条核准过的译名再跑同一次预览，它才跟着一起改，
    /// 并且把用过的模型名记进批次（`04 §7.8` 的审计字段）。
    #[test]
    fn only_an_approved_ai_name_is_applied_and_audited() {
        let base = tree("skip");
        let nor = root_of(&base, "nor");
        std::fs::create_dir(base.join("nor").join("精霊の魔力")).unwrap();
        std::fs::create_dir(base.join("nor").join("名称~甲")).unwrap();
        std::fs::create_dir(base.join("nor").join("名称〜甲")).unwrap();

        let conn = db();
        let rep = apply(&conn, std::slice::from_ref(&nor)).expect("应用");
        assert_eq!((rep.applied, rep.skipped, rep.planned), (0, 3, 0), "撞名的两条谁也不许赢");
        assert_names(&base.join("nor"), &["名称〜甲", "名称~甲", "精霊の魔力"]);
        assert_eq!(count_items(&conn, &rep.batch_id), 0, "一条没成也不该有明细");
        assert_eq!(model_of(&conn, &rep.batch_id), None, "一分钱 token 没花的批次不该记模型");

        let items = preview_level(&nor, Some("nor"), &[Entry { name: "精霊の魔力".into(), is_dir: true }]).0;
        assert!(items[0].needs_ai, "这条该由模型定名，否则本用例根本走不到那条路径");
        record(&conn, "qwen2.5", &[approved(&items[0], "精灵的魔力")]).expect("写库");
        let rep = apply(&conn, &[nor]).expect("第二次应用");
        assert_eq!((rep.applied, rep.skipped, rep.planned), (1, 2, 1), "{:?}", rep.failed);
        assert_names(&base.join("nor"), &["名称〜甲", "名称~甲", "精灵的魔力"]);
        assert_eq!(model_of(&conn, &rep.batch_id).as_deref(), Some("qwen2.5"));
        assert!(batch_row(&conn, &rep.batch_id).3.unwrap().starts_with("本地规则 + 模型译名"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rollback_restores_names_and_redo_replays_them() {
        let base = tree("rollback");
        let nor = root_of(&base, "nor");
        std::fs::create_dir(base.join("nor").join("特典~")).unwrap();
        std::fs::write(base.join("nor").join("歌  手.srt"), "x").unwrap();
        let conn = db();
        let id = apply(&conn, std::slice::from_ref(&nor)).expect("应用").batch_id;

        let back = rollback(&conn, &id).expect("回滚");
        assert_eq!((back.planned, back.applied, back.failed.len()), (2, 2, 0));
        assert_names(&base.join("nor"), &["特典~", "歌  手.srt"]);
        let (applied_at, item_count, rolled_back, note) = batch_row(&conn, &id);
        assert_eq!((item_count, rolled_back), (2, 1), "行不删，只翻旗子");
        assert!(applied_at.is_some(), "applied_at 记的是首次应用，回滚不改它");
        assert!(note.unwrap().starts_with("回滚 · 计划 2 · 成功 2"));

        let again = redo(&conn, &id).expect("重做");
        assert_eq!((again.planned, again.applied), (2, 2));
        assert_names(&base.join("nor"), &["特典～", "歌 手.srt"]);
        assert_eq!(batch_row(&conn, &id).2, 0, "回滚是可回滚的");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rollback_refuses_an_unknown_batch() {
        let conn = db();
        let e = rollback(&conn, "rb不存在").unwrap_err();
        assert!(e.contains("找不到批次"), "{e}");
    }

    #[test]
    fn history_is_appended_on_apply_and_taken_back_on_rollback() {
        let base = tree("history");
        std::fs::write(base.join("rename_history.json"), "{}").unwrap();
        let nor = root_of(&base, "nor");
        std::fs::create_dir(base.join("nor").join("特典~")).unwrap();
        std::fs::write(base.join("nor").join("歌  手.srt"), "x").unwrap();

        let conn = db();
        let rep = apply(&conn, std::slice::from_ref(&nor)).expect("应用");
        assert_eq!(rep.history_written, 2);
        assert!(rep.history_file.unwrap().ends_with("rename_history.json"));
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(base.join("rename_history.json")).unwrap())
                .unwrap();
        // 顶层键 = 实际根目录名；文件夹挂在原始名那一格，躺在根下的文件挂在根名那一格
        assert_eq!(doc["nor"]["特典~"]["_"], "特典～");
        assert_eq!(doc["nor"]["nor"]["歌  手.srt"], "歌 手.srt");

        let back = rollback(&conn, &rep.batch_id).expect("回滚");
        assert_eq!(back.history_written, 2, "撤掉几条也要报数");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(base.join("rename_history.json")).unwrap())
                .unwrap();
        assert_eq!(doc, serde_json::json!({}), "本批写的条目撤完，不留空壳");
        let _ = std::fs::remove_dir_all(&base);
    }
}
