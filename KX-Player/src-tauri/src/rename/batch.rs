//! 改名批次的**账本**：`rename_batches` / `rename_items` 两张表 + 用户既有的 `rename_history.json`。
//!
//! 这里只记账、不动盘 —— 真正改文件系统的是 [`apply`](super::apply)，它每改成一项就回来登记
//! 一行（`04 §7.7` 第 3 条：不攒到最后批量写）。分成两半的理由：账本的正确性可以用内存库证明，
//! 而写盘的那一半要拿真实临时目录才能测，两者的测试代价差一个量级。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

use crate::rename::history::{self, Delta};

pub const FOLDER: &str = "folder";
pub const FILE: &str = "file";

/// 一条改名映射，**永远是正向**（`old` = 预览时盘上的名字，`new` = 改完的名字）。
/// 回滚不新增映射，只是把同一行的 `applied` 翻回 0。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub old: String,
    pub new: String,
    pub level: &'static str,
}

/// 一批的账（`rename_batches` 的单表投影）。`rolled_back` 只改标记不删行 ——
/// 「回滚本身也是可回滚操作」（`04 §7.8`）要靠这些行重做。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub applied_at: Option<i64>,
    pub item_count: i64,
    pub rolled_back: bool,
    pub note: Option<String>,
}

pub fn new_batch_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!("rb{:x}{:x}", now_ms(), SEQ.fetch_add(1, Ordering::SeqCst))
}

/// 批次头一行：`applied_at` 先留空，收尾时补 —— 半路崩了就看得出现在还没有跑完。
/// `provider_id` 恒为 NULL：`ai_providers` 那张表从来没建过（接入信息在设置里，不在库里），
/// 没有可引用的行就不硬造一个。`model` 只在**这批真的用了模型译名**时写 —— 它是审计字段
/// （`04 §7.8`：这批钱花在哪个模型上），本地规则批次一分钱没花，留 NULL（`04 §7.11`）。
pub fn open(conn: &Connection, id: &str, model: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT INTO rename_batches (id, provider_id, model, created_at, item_count, rolled_back, note) \
         VALUES (?1, NULL, ?3, ?2, 0, 0, NULL)",
        params![id, now_ms(), model],
    )
    .map_err(|e| format!("建批次失败：{e}"))?;
    Ok(())
}

/// 收尾（首次应用 / 重做走这条）：填 `applied_at`、真实条目数和审计说明。
/// 「成功/跳过/失败数」进 `note` —— DDL 没给这三项留列，而 `note` 正是为此存在的（`04 §7.8`）。
pub fn finish(
    conn: &Connection,
    id: &str,
    kind: &str,
    planned: usize,
    applied: usize,
    skipped: usize,
    failed: usize,
) -> Result<(), String> {
    let note = format!("{kind} · 计划 {planned} · 成功 {applied} · 跳过 {skipped} · 失败 {failed}");
    conn.execute(
        "UPDATE rename_batches SET applied_at = ?2, rolled_back = 0, note = ?3, \
         item_count = (SELECT COUNT(*) FROM rename_items WHERE batch_id = ?1) WHERE id = ?1",
        params![id, now_ms(), note],
    )
    .map_err(|e| format!("收尾批次失败：{e}"))?;
    Ok(())
}

/// 回滚与重做只翻标记：条目行一条都不删，行数与映射才是「这一批动过哪些」的完整答案。
pub fn mark_rolled_back(conn: &Connection, id: &str, rolled_back: bool, note: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE rename_batches SET rolled_back = ?2, note = ?3 WHERE id = ?1",
        params![id, i64::from(rolled_back), note],
    )
    .map_err(|e| format!("更新批次状态失败：{e}"))?;
    Ok(())
}

/// 一行的 `applied` 旗子。键始终是库里的 `old_path`（正向映射的那一侧），
/// 命中数不是 1 就报错 —— 明细对不上还继续改盘，等于把一批文件交给一份不可信的账。
pub fn set_item(conn: &Connection, batch_id: &str, p: &Pair, applied: bool) -> Result<(), String> {
    let n = conn
        .execute(
            "UPDATE rename_items SET applied = ?3 WHERE batch_id = ?1 AND old_path = ?2",
            params![batch_id, p.old, i64::from(applied)],
        )
        .map_err(|e| format!("记账失败（{e}）"))?;
    if n == 1 {
        Ok(())
    } else {
        Err(format!("明细对不上（命中 {n} 行）"))
    }
}

/// 改盘成功之后**立刻**写这一行（`04 §7.7` 第 3 条）
pub fn add_item(conn: &Connection, batch_id: &str, p: &Pair) -> Result<(), String> {
    conn.execute(
        "INSERT INTO rename_items (batch_id, old_path, new_path, level, applied) VALUES (?1, ?2, ?3, ?4, 1)",
        params![batch_id, p.old, p.new, p.level],
    )
    .map_err(|e| format!("记账失败（{e}）"))?;
    Ok(())
}

/// 取这一批的映射。`applied`：1 = 现在盘上是新名，0 = 已回滚/未生效。
pub fn pairs(conn: &Connection, batch_id: &str, applied: i64) -> Result<Vec<Pair>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT old_path, new_path, level FROM rename_items WHERE batch_id = ?1 AND applied = ?2",
        )
        .map_err(|e| e.to_string())?;
    let out = stmt
        .query_map(params![batch_id, applied], |r| {
            let (old, new, level) = (r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?);
            Ok(Pair { old, new, level: if level == FOLDER { FOLDER } else { FILE } })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 批次列表（UI 靠它挑哪一批回滚）。按创建时间倒序，同一毫秒内的按 id 倒序 ——
/// id 里带毫秒与序号，这样同一批连点两次也不会让顺序来回跳。
pub fn list(conn: &Connection, limit: i64) -> Result<Vec<Batch>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, provider_id, model, created_at, applied_at, item_count, rolled_back, note \
             FROM rename_batches ORDER BY created_at DESC, id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let out = stmt
        .query_map(params![limit], |r| {
            Ok(Batch {
                id: r.get(0)?,
                provider_id: r.get(1)?,
                model: r.get(2)?,
                created_at: r.get(3)?,
                applied_at: r.get(4)?,
                item_count: r.get(5)?,
                rolled_back: r.get::<_, i64>(6)? != 0,
                note: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 这一批到底存不存在（回滚/重做拿一个不存在的 id 时必须报错，而不是「成功 0 条」）
pub fn exists(conn: &Connection, id: &str) -> Result<bool, String> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM rename_batches WHERE id = ?1", params![id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// 把本批生效的映射追加进（`undo = false`）或从（`undo = true`）既有的
/// `rename_history.json` 里撤掉（`04 §7.1` 第 2 条 + `04 §7.8` 回滚第三条）。
///
/// 返回 `(落到哪个文件, 实际改动条数)`；那一层没有这份文件就是 `(None, 0)` —— **不去别处
/// 凭空造一个同名账本**，那只会让人误以为自己的那份已经更新过了。
pub fn sync_history(roots: &[String], applied: &[Pair], undo: bool) -> Result<(Option<String>, usize), String> {
    let paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    let Some(file) = history::locate(&paths) else {
        return Ok((None, 0));
    };
    let mut doc = history::load(&file)?;
    let deltas = deltas_of(&doc, &paths, applied);
    let n = if undo {
        history::revert(&mut doc, &deltas)
    } else {
        history::merge(&mut doc, &deltas)?
    };
    if n > 0 {
        history::store(&file, &doc)?;
    }
    Ok((Some(file.to_string_lossy().into_owned()), n))
}

/// 生效的映射 → history 条目。顶层键 = **实际根目录名**（`04 §7.1` 第 4 条），
/// 二级键 = 作品文件夹的原始名（多轮改名折叠进同一条，第 6 条）。
fn deltas_of(doc: &Value, roots: &[PathBuf], applied: &[Pair]) -> Vec<Delta> {
    let mut out = Vec::new();
    for p in applied {
        let from = Path::new(&p.old);
        // 所选根相互嵌套时取**最外层**的那个：顶层键要说的是「用户勾的那一层叫什么」，
        // 而作品文件夹正是它下面的一条（内层根同时也是一条待改名的条目，不该当成顶层）
        let Some(root) = roots
            .iter()
            .filter(|r| from.starts_with(r.as_path()))
            .min_by_key(|r| r.components().count())
        else {
            continue;
        };
        let Some(root_name) = root.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let (Some(name), Some(to_name)) = (
            from.file_name().and_then(|s| s.to_str()),
            Path::new(&p.new).file_name().and_then(|s| s.to_str()),
        ) else {
            continue;
        };
        // 文件夹条目说的就是它自己；文件条目说的是它所属的那层，直接躺在根下的文件所属即根
        let current = if p.level == FOLDER {
            name.to_string()
        } else {
            child_of_root(from, root).unwrap_or_else(|| root_name.to_string())
        };
        let group = history::group_key(doc, root_name, &current);
        // 这一格本批之前是什么：回滚要回填它（第二轮改名的回滚不能把第一轮的记录一起抹掉）
        let prev = doc
            .get(root_name)
            .and_then(|g| g.get(&group))
            .and_then(|e| e.get(if p.level == FOLDER { "_" } else { name }))
            .and_then(Value::as_str)
            .filter(|v| *v != to_name)
            .map(str::to_string);
        out.push(Delta {
            root: root_name.to_string(),
            group,
            key: if p.level == FOLDER { "_".into() } else { name.to_string() },
            to: to_name.to_string(),
            prev,
        });
    }
    out
}

/// 文件所属的那层作品目录名。一层预览时文件直接躺在根下 —— 那就没有中间层，返回 `None`
/// 让调用方退回用根名当组键（`04 §7.1` 第 4 条只规定顶层键，没规定这种文件该挂哪儿）。
fn child_of_root(from: &Path, root: &Path) -> Option<String> {
    if from.parent()? == root {
        return None;
    }
    let rest = from.strip_prefix(root).ok()?;
    Some(rest.components().next()?.as_os_str().to_string_lossy().into_owned())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    /// 表由**真迁移**建，不是手抄的 DDL —— 忘了往 `MIGRATIONS` 里加那一行，这里就该红
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::migrations::apply(&conn).expect("建表");
        conn
    }

    fn pair(old: &str, new: &str, level: &'static str) -> Pair {
        Pair { old: old.into(), new: new.into(), level }
    }

    fn read_doc(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn batch_ids_do_not_collide_within_the_same_millisecond() {
        let ids: Vec<String> = (0..64).map(|_| new_batch_id()).collect();
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 64);
    }

    #[test]
    fn applied_is_a_flag_not_a_delete_and_the_detail_must_line_up() {
        let conn = db();
        open(&conn, "b1", None).unwrap();
        let p = pair("L:/nor/特典~", "L:/nor/特典～", FOLDER);
        add_item(&conn, "b1", &p).unwrap();
        // 同一条改名的 old_path 重复登记：主键挡下来，而不是留下两行互相矛盾的账
        assert!(add_item(&conn, "b1", &p).unwrap_err().contains("记账失败"));
        assert_eq!(pairs(&conn, "b1", 1).unwrap(), vec![p.clone()]);

        set_item(&conn, "b1", &p, false).unwrap();
        assert!(pairs(&conn, "b1", 1).unwrap().is_empty(), "行不删，只翻旗子");
        assert_eq!(pairs(&conn, "b1", 0).unwrap(), vec![p.clone()], "level 要读回成那两个常量之一");
        // 库里没有这一行却偏要翻它的旗子：报错，不能默默改盘（命中数≠1 就是账与盘对不上）
        let e = set_item(&conn, "b1", &pair("L:/nor/没登记过", "L:/nor/x", FILE), false).unwrap_err();
        assert!(e.contains("明细对不上"), "{e}");
        assert!(exists(&conn, "b1").unwrap() && !exists(&conn, "b2").unwrap());
    }

    #[test]
    fn finish_counts_the_rows_it_has_rather_than_the_number_it_was_told() {
        let conn = db();
        open(&conn, "b1", None).unwrap();
        add_item(&conn, "b1", &pair("L:/a~", "L:/a", FILE)).unwrap();
        add_item(&conn, "b1", &pair("L:/b~", "L:/b", FOLDER)).unwrap();
        finish(&conn, "b1", "本地规则", 2, 2, 0, 0).unwrap();
        open(&conn, "b2", None).unwrap();
        finish(&conn, "b2", "本地规则", 0, 0, 3, 0).unwrap();

        let rows = list(&conn, 10).unwrap();
        assert_eq!(rows.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["b2", "b1"]);
        let b1 = rows.iter().find(|b| b.id == "b1").unwrap();
        assert_eq!(b1.item_count, 2, "item_count 是行数，不是传进来的数");
        assert!(b1.applied_at.is_some());
        assert_eq!(b1.note.as_deref(), Some("本地规则 · 计划 2 · 成功 2 · 跳过 0 · 失败 0"));
        let b2 = rows.iter().find(|b| b.id == "b2").unwrap();
        assert_eq!(
            (b2.item_count, b2.applied_at.is_some(), b2.provider_id.is_none(), b2.model.is_none()),
            (0, true, true, true),
            "纯本地批次一分钱没花，两个审计字段都留 NULL"
        );

        // 回滚只换旗子与说明：条目行、首次应用时间都不动（重做要靠这些行）
        mark_rolled_back(&conn, "b1", true, "回滚 · 计划 2 · 成功 2").unwrap();
        let after = list(&conn, 10).unwrap();
        let b1 = after.iter().find(|b| b.id == "b1").unwrap();
        assert_eq!((b1.rolled_back, b1.item_count, b1.note.as_deref()), (true, 2, Some("回滚 · 计划 2 · 成功 2")));
        assert_eq!(b1.applied_at, rows.iter().find(|b| b.id == "b1").unwrap().applied_at);
    }

    #[test]
    fn deltas_fold_into_the_original_group_and_remember_what_was_there() {
        let doc = json!({ "nor": { "旧甲": { "_": "新甲" } } });
        let roots = [PathBuf::from("L:/x/nor")];
        // 第一轮记的是 旧甲→新甲，本批把 新甲 改成 更甲：组键仍是原始名，且要记住 prev
        let d = deltas_of(&doc, &roots, &[pair("L:/x/nor/新甲", "L:/x/nor/更甲", FOLDER)]);
        assert_eq!((d[0].group.as_str(), d[0].key.as_str()), ("旧甲", "_"));
        assert_eq!(d[0].prev.as_deref(), Some("新甲"), "回滚要回填第一轮的结果");

        // 根相互嵌套时顶层键取**最外层**那一个，作品文件夹挂在它下面的一格里
        let nested = [PathBuf::from("L:/x/nor"), PathBuf::from("L:/x/nor/特典~")];
        let d = deltas_of(&doc, &nested, &[pair("L:/x/nor/特典~/歌  手.srt", "L:/x/nor/特典~/歌 手.srt", FILE)]);
        assert_eq!(
            (d[0].root.as_str(), d[0].group.as_str(), d[0].key.as_str()),
            ("nor", "特典~", "歌  手.srt"),
            "文件挂在所属作品那一格，不是挂在内层根自己名下"
        );
        // 直接躺在根下的文件没有中间层 → 退回根名那一格
        let d = deltas_of(&doc, &roots, &[pair("L:/x/nor/歌  手.srt", "L:/x/nor/歌 手.srt", FILE)]);
        assert_eq!((d[0].group.as_str(), d[0].key.as_str()), ("nor", "歌  手.srt"));
        // 一个根都不认的路径进不了这份账：宁可少写，也不要写到一个没人认识的地方
        assert!(deltas_of(&doc, &roots, &[pair("L:/别的/甲~", "L:/别的/甲", FOLDER)]).is_empty());
    }

    #[test]
    fn history_is_written_only_where_the_user_already_keeps_one() {
        let base = std::env::temp_dir().join(format!("kx-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("nor")).unwrap();
        let root = base.join("nor").to_string_lossy().into_owned();
        let p = pair(&format!("{root}/特典~"), &format!("{root}/特典～"), FOLDER);

        let roots = std::slice::from_ref(&root);
        assert_eq!(sync_history(roots, std::slice::from_ref(&p), false).unwrap(), (None, 0));
        assert!(!base.join(history::HISTORY_FILE).exists(), "那一层没有这份账，就不能凭空造一个");

        std::fs::write(base.join(history::HISTORY_FILE), "{}").unwrap();
        let (file, n) = sync_history(roots, std::slice::from_ref(&p), false).unwrap();
        assert_eq!((n, file.is_some()), (1, true));
        let hist = base.join(history::HISTORY_FILE);
        assert_eq!(read_doc(&hist)["nor"]["特典~"]["_"], "特典～");
        // 同一条重复追加：值本来就是这样，不该再算一次改动（也不该重写文件）
        assert_eq!(sync_history(roots, std::slice::from_ref(&p), false).unwrap().1, 0);
        assert_eq!(sync_history(roots, &[p], true).unwrap().1, 1);
        assert_eq!(read_doc(&hist), json!({}), "撤干净之后不留空壳");
        assert!(!base.join(format!("{}.tmp", history::HISTORY_FILE)).exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
